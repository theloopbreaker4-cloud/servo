/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! CSS Counter implementation.
//!
//! Implements the CSS counter algorithms described in:
//! <https://drafts.csswg.org/css-lists-3/#auto-numbering>
//!
//! The algorithm works as a pre-order, depth-first DOM traversal that builds
//! a snapshot of counter values for each element/pseudo-element node.
//! These snapshots are stored in a map keyed by OpaqueNode and later looked
//! up when generating `content: counter(name)` text.

use std::collections::HashMap;

use layout_api::{LayoutElement, LayoutNode};
use script::layout_dom::ServoLayoutNode;
use style::Atom;
use style::counter_style::CounterStyle;
use style::dom::OpaqueNode;
use style::values::CustomIdent;
use style::values::computed::counters::{CounterIncrement, CounterReset};

use crate::context::LayoutContext;
use crate::lists::generate_counter_value;

/// A single counter scope entry: a counter with its current value.
#[derive(Clone, Debug)]
struct CounterEntry {
    /// Counter name.
    name: CustomIdent,
    /// Current counter value.
    value: i32,
    /// Whether this is a reversed counter (from `counter-reset: reversed(name)`).
    is_reversed: bool,
}

/// Counter scope stack — one level per element that creates new counter scope.
/// Per spec, counters are scoped: a child creates a new scope, the parent's
/// counter continues alongside.
///
/// We track the *innermost* counter with a given name — that's what `counter()` resolves to.
#[derive(Clone, Debug, Default)]
pub(crate) struct CounterScope {
    entries: Vec<CounterEntry>,
}

impl CounterScope {
    fn find_mut(&mut self, name: &CustomIdent) -> Option<&mut CounterEntry> {
        // innermost matching entry (last in the list)
        self.entries.iter_mut().rev().find(|e| e.name == *name)
    }

    fn find(&self, name: &CustomIdent) -> Option<&CounterEntry> {
        self.entries.iter().rev().find(|e| e.name == *name)
    }

    /// Get value of a named counter, or 0 if not found.
    pub fn get(&self, name: &CustomIdent) -> i32 {
        self.find(name).map(|e| e.value).unwrap_or(0)
    }

    /// Get all values of a named counter (for `counters()` — nested scopes).
    pub fn get_all(&self, name: &CustomIdent) -> Vec<i32> {
        self.entries
            .iter()
            .filter(|e| e.name == *name)
            .map(|e| e.value)
            .collect()
    }
}

/// Per-node counter snapshot — the state of all counters at the moment
/// a pseudo-element's `content` is evaluated.
#[derive(Clone, Debug, Default)]
pub(crate) struct CounterSnapshot(pub CounterScope);

/// Global counter state shared across the traversal (single-threaded box tree construction).
/// Maps each node's OpaqueNode to its counter snapshot.
pub(crate) struct CounterState {
    /// Counter scope stack during traversal.
    scope_stack: Vec<CounterScope>,
    /// Snapshots: node → counter scope at the point when ::before was processed.
    pub snapshots: HashMap<OpaqueNode, CounterSnapshot>,
}

impl CounterState {
    pub fn new() -> Self {
        let mut root_scope = CounterScope::default();
        // The `list-item` counter is implicitly defined at the root.
        root_scope.entries.push(CounterEntry {
            name: CustomIdent(Atom::from("list-item")),
            value: 0,
            is_reversed: false,
        });
        Self {
            scope_stack: vec![root_scope],
            snapshots: HashMap::new(),
        }
    }

    fn current_scope(&self) -> &CounterScope {
        self.scope_stack.last().expect("scope stack is never empty")
    }

    fn current_scope_mut(&mut self) -> &mut CounterScope {
        self.scope_stack.last_mut().expect("scope stack is never empty")
    }

    /// Push a new scope (child element begins).
    fn push_scope(&mut self) {
        let parent = self.current_scope().clone();
        self.scope_stack.push(parent);
    }

    /// Pop scope (child element ends).
    fn pop_scope(&mut self) {
        if self.scope_stack.len() > 1 {
            self.scope_stack.pop();
        }
    }

    /// Apply `counter-reset` — creates new counter instances in the current scope.
    fn apply_reset(&mut self, reset: &CounterReset) {
        let scope = self.current_scope_mut();
        for pair in reset.iter() {
            // counter-reset creates a NEW instance (push to the scope's list).
            // Per spec, it replaces any existing counter with the same name in this scope level,
            // but since we clone the parent scope, we just push a new one which shadows the parent.
            // Remove any existing counter with same name that was inherited from parent (last pushed).
            // Actually per spec: counter-reset always creates a new counter in THIS element's scope.
            scope.entries.push(CounterEntry {
                name: pair.name.clone(),
                value: pair.value,
                is_reversed: pair.is_reversed,
            });
        }
    }

    /// Apply `counter-increment` — increments the innermost named counter.
    fn apply_increment(&mut self, increment: &CounterIncrement) {
        let scope = self.current_scope_mut();
        for pair in increment.iter() {
            if let Some(entry) = scope.find_mut(&pair.name) {
                entry.value = entry.value.saturating_add(pair.value);
            } else {
                // If counter doesn't exist, create one starting at increment value.
                scope.entries.push(CounterEntry {
                    name: pair.name.clone(),
                    value: pair.value,
                    is_reversed: false,
                });
            }
        }
    }

    /// Snapshot current scope for a node (used for ::before / ::after lookups).
    fn snapshot_for(&mut self, node: OpaqueNode) {
        let snapshot = CounterSnapshot(self.current_scope().clone());
        self.snapshots.insert(node, snapshot);
    }

    /// Get counter value for a node.
    pub fn counter_value(&self, node: OpaqueNode, name: &CustomIdent) -> i32 {
        self.snapshots
            .get(&node)
            .map(|s| s.0.get(name))
            .unwrap_or(0)
    }

    /// Get all counter values for a node (for `counters()`).
    pub fn counter_all_values(&self, node: OpaqueNode, name: &CustomIdent) -> Vec<i32> {
        self.snapshots
            .get(&node)
            .map(|s| s.0.get_all(name))
            .unwrap_or_default()
    }
}

/// Walk the DOM tree building the counter state.
/// This is a pre-order traversal matching the counter algorithm in the spec.
pub(crate) fn build_counter_state(
    context: &LayoutContext,
    root: ServoLayoutNode<'_>,
    state: &mut CounterState,
) {
    walk_node(&context.style_context, root, state);
}

fn walk_node<'dom>(
    style_context: &style::context::SharedStyleContext<'_>,
    node: ServoLayoutNode<'dom>,
    state: &mut CounterState,
) {
    // Only process element nodes (skip text nodes, comments, etc.)
    let element = match node.as_element() {
        Some(e) => e,
        None => return,
    };

    // Skip unstyled nodes — style() panics if style_data is None.
    // This happens for <script>, <head>, shadow DOM internals, etc.
    if element.style_data().is_none() {
        return;
    }

    let style = node.style(style_context);

    // Check if display:none — those don't participate in counters.
    // display:none elements still process counter-reset but not counter-increment.
    // Simplified: skip entirely for display:none.
    // TODO: display:none should process counter-reset according to latest spec.
    use crate::style_ext::Display as LayoutDisplay;
    let is_display_none = matches!(
        LayoutDisplay::from(style.get_box().display),
        LayoutDisplay::None
    );

    // Push new scope for this element.
    state.push_scope();

    if !is_display_none {
        // 1. Apply counter-reset (creates new counters).
        let reset = style.clone_counter_reset();
        if !reset.is_empty() {
            state.apply_reset(&reset);
        }

        // 2. Apply counter-increment.
        let increment = style.clone_counter_increment();
        let list_item_atom = Atom::from("list-item");
        let has_list_item_increment = increment.iter().any(|p| p.name.0 == list_item_atom);
        if !increment.is_empty() {
            state.apply_increment(&increment);
        }

        // Handle list-item auto-increment.
        // <https://drafts.csswg.org/css-lists-3/#list-item-counter>
        // Auto-increment only if counter-increment doesn't already mention list-item.
        if !has_list_item_increment {
            // list-item counter is auto-incremented for elements that are list items.
            // We check the display type: list-item generates a ::marker.
            if style.get_box().display.is_list_item() {
                let list_item_name = CustomIdent(Atom::from("list-item"));
                let scope = state.current_scope_mut();
                if let Some(entry) = scope.find_mut(&list_item_name) {
                    if entry.is_reversed {
                        entry.value = entry.value.saturating_sub(1);
                    } else {
                        entry.value = entry.value.saturating_add(1);
                    }
                }
            }
        }
    }

    // Snapshot for this element's node (used by ::before and regular content).
    let opaque = node.opaque();
    state.snapshot_for(opaque);

    // Traverse children.
    if !is_display_none {
        for child in node.flat_tree_children() {
            walk_node(style_context, child, state);
        }
    }

    // Pop scope when leaving element.
    state.pop_scope();
}

/// Generate a counter string representation from a value and style.
/// <https://drafts.csswg.org/css-counter-styles-3/#generate-a-counter>
pub(crate) fn render_counter(value: i32, style: &CounterStyle) -> String {
    generate_counter_value(value, style)
}

/// Generate a `counter(name)` value for a node.
pub(crate) fn resolve_counter(
    counter_state: &CounterState,
    node: OpaqueNode,
    name: &CustomIdent,
    style: &CounterStyle,
) -> String {
    let value = counter_state.counter_value(node, name);
    render_counter(value, style)
}

/// Generate a `counters(name, separator)` value for a node.
pub(crate) fn resolve_counters(
    counter_state: &CounterState,
    node: OpaqueNode,
    name: &CustomIdent,
    separator: &str,
    style: &CounterStyle,
) -> String {
    let values = counter_state.counter_all_values(node, name);
    if values.is_empty() {
        return render_counter(0, style);
    }
    values
        .iter()
        .map(|&v| render_counter(v, style))
        .collect::<Vec<_>>()
        .join(separator)
}

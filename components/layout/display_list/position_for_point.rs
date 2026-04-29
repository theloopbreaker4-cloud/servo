/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Aurora: pixel → text position resolution for body-text selection.
//!
//! Servo's existing `hit_test` returns "this fragment was hit, here's the
//! DOM node". For text selection we need a finer resolution: given a
//! pixel inside a text fragment, return WHICH CHARACTER in the run the
//! pixel lies over. This module mirrors the structure of `hit_test.rs`
//! but, instead of pushing an `ElementsFromPointResult`, returns
//! `Option<TextHit>` from the topmost text fragment under the cursor.
//!
//! The glyph-walk loop is the inverse of the selection rendering loop in
//! `display_list/mod.rs` around line 1045 — that one walks the run with
//! a known character range and accumulates pixel advance; this one walks
//! with a known pixel and accumulates the character index.
//!
//! Phase 1 only — viewport-relative, single-fragment resolution. We do
//! NOT yet snap to nearest line on miss (Blink's `LayoutBlockFlow::
//! PositionForPoint` does that); we return None if the cursor is not
//! directly over a glyph. Selection drag will work as long as the user
//! stays over text content.

use app_units::Au;
use euclid::num::Zero;
use style::dom::OpaqueNode;
use webrender_api::units::LayoutPoint;

use crate::display_list::{StackingContext, StackingContextContent, StackingContextTree};
use crate::fragment_tree::{Fragment, TextFragment};
use crate::geom::PhysicalRect;

/// Result of a successful pixel → text-position lookup.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextHit {
    /// The DOM node owning the hit text fragment.
    pub node: OpaqueNode,
    /// Zero-based UTF-16-code-unit offset of the character whose glyph
    /// the cursor is over. Suitable as the `offset` of a DOM `Range`
    /// endpoint when the start/end node is a text node.
    pub char_offset: usize,
}

impl StackingContextTree {
    /// Walk the stacking-context tree for the topmost text fragment whose
    /// inked rect contains `point` (CSS pixels in viewport coordinates),
    /// then return the character offset within that fragment.
    pub(crate) fn position_for_point(&self, point: LayoutPoint) -> Option<TextHit> {
        let mut search = PositionSearch {
            point,
            result: None,
        };
        // Walk in the same direction `hit_test` walks — reverse stacking
        // order — so the topmost text wins on overlap.
        self.root_stacking_context.position_for_point(&mut search);
        search.result
    }
}

struct PositionSearch {
    point: LayoutPoint,
    result: Option<TextHit>,
}

impl StackingContext {
    fn position_for_point(&self, search: &mut PositionSearch) {
        // Mirror StackingContext::hit_test ordering: descendants in
        // reverse so topmost paints first.
        for content in self.contents.iter().rev() {
            if search.result.is_some() {
                return;
            }
            match content {
                StackingContextContent::Fragment {
                    containing_block,
                    fragment,
                    ..
                } => {
                    fragment.position_for_point(search, containing_block);
                },
                StackingContextContent::AtomicInlineStackingContainer { .. } => {
                    // Atomic inline replaced elements (images, iframes)
                    // don't carry text — skip them.
                },
            }
        }
    }
}

impl Fragment {
    fn position_for_point(
        &self,
        search: &mut PositionSearch,
        containing_block: &PhysicalRect<Au>,
    ) {
        let Fragment::Text(text) = self else { return };
        let text = text.borrow();

        // Translate the fragment-local rect into viewport coordinates the
        // caller is testing against.
        let fragment_rect = text.base.rect.translate(containing_block.origin.to_vector());
        let rect_origin_x_px = fragment_rect.origin.x.to_f32_px();
        let rect_origin_y_px = fragment_rect.origin.y.to_f32_px();
        let rect_w_px = fragment_rect.size.width.to_f32_px();
        let rect_h_px = fragment_rect.size.height.to_f32_px();

        // Y-axis check first — much cheaper than the glyph walk, and
        // most fragments are off the cursor's line.
        if search.point.y < rect_origin_y_px ||
            search.point.y > rect_origin_y_px + rect_h_px
        {
            return;
        }
        // X-axis — fragment must contain the point or it's the wrong run.
        if search.point.x < rect_origin_x_px ||
            search.point.x > rect_origin_x_px + rect_w_px
        {
            return;
        }

        let Some(tag) = text.base.tag else { return };
        let target_x_in_fragment = Au::from_f32_px(search.point.x - rect_origin_x_px);
        let char_offset = char_offset_at_advance(&text, target_x_in_fragment);

        search.result = Some(TextHit {
            node: tag.node,
            char_offset,
        });
    }
}

/// Walk the fragment's glyph stores accumulating `current_advance` until
/// it crosses `target_x`, then return the character index reached.
///
/// This is the inverse of the selection-rendering loop in
/// `display_list/mod.rs:1045-1079`: that one walks a known character
/// range to compute the pixel rect; we walk a known pixel to find the
/// character.
///
/// Handles glyph clusters (one glyph mapping to multiple chars and vice
/// versa) by stepping `character_count` per glyph instead of always 1.
/// Justification adjustment is applied per word-separator glyph to match
/// the rendering path.
fn char_offset_at_advance(fragment: &TextFragment, target_x: Au) -> usize {
    let justification_adjustment = fragment.justification_adjustment;
    let mut current_character_index: usize = 0;
    let mut current_advance = Au::zero();

    for glyph_store in fragment.glyphs.iter() {
        // Cheap fast-forward: if the entire run ends before target_x, skip
        // it whole. Mirrors the same shortcut in the rendering loop.
        let run_total = glyph_store.total_advance() +
            (justification_adjustment * glyph_store.total_word_separators() as i32);
        if current_advance + run_total < target_x {
            current_advance += run_total;
            current_character_index += glyph_store.total_characters() as usize;
            continue;
        }

        for glyph in glyph_store.glyphs() {
            let mut glyph_advance = glyph.advance();
            if glyph.char_is_word_separator() {
                glyph_advance += justification_adjustment;
            }

            // If this glyph's pixel range straddles target_x, the cursor
            // is on this glyph. Decide which side of the glyph midpoint
            // the cursor is on so a click on the right half snaps to the
            // NEXT character — matches WebKit/Blink behavior where
            // selection caret prefers the gap closer to the cursor.
            let glyph_end = current_advance + glyph_advance;
            if target_x < glyph_end {
                let glyph_mid = current_advance + (glyph_advance / 2);
                let chars_in_glyph = glyph.character_count() as usize;
                if target_x <= glyph_mid {
                    return current_character_index;
                } else {
                    return current_character_index + chars_in_glyph;
                }
            }

            current_advance = glyph_end;
            current_character_index += glyph.character_count() as usize;
        }
    }

    // Cursor is past the end of all glyphs — return end-of-fragment.
    current_character_index
}

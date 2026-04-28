/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Aurora-themed native scrollbar rendering.
//!
//! Servo's upstream painter does not draw scrollbars (TODO at
//! `script/dom/window.rs:1956`). This module fills that gap by appending
//! WebRender quads to the per-webview reference frame's display list,
//! sized and positioned from the scroll tree's content/clip rects and
//! offset. Quads are emitted in CSS pixels — the reference frame's
//! transform handles HiDPI / page-zoom / pinch-zoom scaling for us.
//!
//! Design: vertical thumb on the right edge of every scrollable area,
//! 8px wide, Aurora teal (#3ca0a0) on a faint dark track. Rendered only
//! when content exceeds clip on a given axis. No interactivity yet —
//! Phase 1 is visual only; mouse drag arrives in a follow-up.

use paint_api::display_list::{ScrollTree, SpatialTreeNodeInfo};
use webrender_api::ExternalScrollId;
use webrender_api::units::{LayoutPoint, LayoutRect, LayoutSize, LayoutVector2D};
use webrender_api::{
    ClipChainId, ColorF, CommonItemProperties, DisplayListBuilder, PrimitiveFlags, SpatialId,
};

/// Scrollbar visual constants — Aurora design language.
const THICKNESS: f32 = 8.0;
const MIN_THUMB_LENGTH: f32 = 24.0;
/// #0c1118 at 35% — darker track for both light and dark pages.
const TRACK_COLOR: ColorF = ColorF { r: 0.047, g: 0.067, b: 0.094, a: 0.35 };
/// #2a7373 at 75% — muted Aurora teal thumb (less saturated than brand).
const THUMB_COLOR: ColorF = ColorF { r: 0.165, g: 0.451, b: 0.451, a: 0.75 };

/// Walk a pipeline's scroll tree and emit scrollbar quads (in CSS pixels)
/// for every scrollable node whose content exceeds its clip rect. Caller
/// supplies the reference-frame's `spatial_id` and `clip_chain_id` so the
/// quads inherit the iframe's transform and clipping.
/// Minimum size of a scrollable area before we bother painting a scrollbar.
/// Below this, the scrollbar would be larger than the viewable region or
/// take more visual weight than the content (common on overflow:scroll
/// heavy sites — GitHub has 60+ small scrollables most of which are
/// invisible UI scaffolding).
const MIN_SCROLLABLE_DIMENSION: f32 = 120.0;

/// Pixel band on each window edge reserved for OS / shell resize handles.
/// Both the drawn thumb position AND the hit-test exclude this band, so
/// users dragging the right or bottom edge of the window get the resize
/// cursor instead of grabbing our scrollbar.
const EDGE_INSET: f32 = 6.0;

pub(crate) fn draw_scrollbars(
    builder: &mut DisplayListBuilder,
    spatial_id: SpatialId,
    clip_chain_id: ClipChainId,
    scroll_tree: &ScrollTree,
) {
    for node in scroll_tree.nodes.iter() {
        let SpatialTreeNodeInfo::Scroll(scrollable) = &node.info else { continue };

        let viewport = scrollable.clip_rect;
        let vp_w = viewport.size().width;
        let vp_h = viewport.size().height;

        // Skip tiny scrollables — they're typically internal UI affordances
        // (autocomplete dropdowns, tag pickers) where a scrollbar would
        // dominate the element. Real content scrollers are larger.
        if vp_w < MIN_SCROLLABLE_DIMENSION || vp_h < MIN_SCROLLABLE_DIMENSION {
            continue;
        }

        let content_w = scrollable.content_rect.size().width;
        let content_h = scrollable.content_rect.size().height;
        let scroll_x = scrollable.offset.x;
        let scroll_y = scrollable.offset.y;

        draw_axis_y(builder, spatial_id, clip_chain_id, viewport, content_h, scroll_y);
        draw_axis_x(builder, spatial_id, clip_chain_id, viewport, content_w, scroll_x);
    }
}

fn draw_axis_y(
    builder: &mut DisplayListBuilder,
    spatial_id: SpatialId,
    clip_chain_id: ClipChainId,
    viewport: LayoutRect,
    content_h: f32,
    scroll_y: f32,
) {
    let viewport_h = viewport.size().height;
    if content_h <= viewport_h + 0.5 {
        return; // nothing to scroll on this axis
    }

    // Inset from the right edge so the OS resize cursor wins on the very
    // last few pixels of the window.
    let track_right = viewport.max.x - EDGE_INSET;
    let track = LayoutRect::from_origin_and_size(
        LayoutPoint::new(track_right - THICKNESS, viewport.min.y),
        LayoutSize::new(THICKNESS, viewport_h),
    );
    push_quad(builder, spatial_id, clip_chain_id, track, TRACK_COLOR);

    let raw_thumb = viewport_h * (viewport_h / content_h);
    let thumb_h = raw_thumb.max(MIN_THUMB_LENGTH).min(viewport_h);
    let max_scroll = (content_h - viewport_h).max(1.0);
    let max_thumb_y = viewport_h - thumb_h;
    let thumb_y = (scroll_y / max_scroll).clamp(0.0, 1.0) * max_thumb_y;

    let thumb = LayoutRect::from_origin_and_size(
        LayoutPoint::new(track_right - THICKNESS, viewport.min.y + thumb_y),
        LayoutSize::new(THICKNESS, thumb_h),
    );
    push_quad(builder, spatial_id, clip_chain_id, thumb, THUMB_COLOR);
}

fn draw_axis_x(
    builder: &mut DisplayListBuilder,
    spatial_id: SpatialId,
    clip_chain_id: ClipChainId,
    viewport: LayoutRect,
    content_w: f32,
    scroll_x: f32,
) {
    let viewport_w = viewport.size().width;
    if content_w <= viewport_w + 0.5 {
        return;
    }

    // Reserve space for the vertical scrollbar if it's also visible
    // and for the bottom-right resize handle.
    let usable_w = viewport_w - THICKNESS - EDGE_INSET;
    let track_bottom = viewport.max.y - EDGE_INSET;

    let track = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.min.x, track_bottom - THICKNESS),
        LayoutSize::new(usable_w, THICKNESS),
    );
    push_quad(builder, spatial_id, clip_chain_id, track, TRACK_COLOR);

    let raw_thumb = usable_w * (usable_w / content_w);
    let thumb_w = raw_thumb.max(MIN_THUMB_LENGTH).min(usable_w);
    let max_scroll = (content_w - viewport_w).max(1.0);
    let max_thumb_x = usable_w - thumb_w;
    let thumb_x = (scroll_x / max_scroll).clamp(0.0, 1.0) * max_thumb_x;

    let thumb = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.min.x + thumb_x, track_bottom - THICKNESS),
        LayoutSize::new(thumb_w, THICKNESS),
    );
    push_quad(builder, spatial_id, clip_chain_id, thumb, THUMB_COLOR);
}

fn push_quad(
    builder: &mut DisplayListBuilder,
    spatial_id: SpatialId,
    clip_chain_id: ClipChainId,
    rect: LayoutRect,
    color: ColorF,
) {
    let properties = CommonItemProperties {
        clip_rect: rect,
        spatial_id,
        clip_chain_id,
        flags: PrimitiveFlags::default(),
    };
    builder.push_rect(&properties, rect, color);
}

// ── Hit-testing & drag arithmetic ────────────────────────────────────────
//
// These helpers let the painter decide whether a mouse event should be
// consumed by a scrollbar (drag thumb / click track) or forwarded to the
// page. They mirror the geometry decisions in `draw_axis_*` exactly so
// hit-testing always agrees with what's drawn on screen.

/// Result of hit-testing a CSS-pixel point against the visible scrollbars
/// of one pipeline's scroll tree.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ScrollbarHit {
    /// The point is on a vertical thumb. Caller can begin drag.
    ThumbY {
        external_id: ExternalScrollId,
        thumb_y: f32,
        thumb_h: f32,
        viewport_min_y: f32,
        viewport_h: f32,
        content_h: f32,
    },
    /// The point is on a horizontal thumb.
    ThumbX {
        external_id: ExternalScrollId,
        thumb_x: f32,
        thumb_w: f32,
        viewport_min_x: f32,
        viewport_w: f32,
        content_w: f32,
    },
    /// The point is on a vertical track but outside the thumb. Caller
    /// should page-scroll toward the click.
    TrackY {
        external_id: ExternalScrollId,
        click_y: f32,
        thumb_y: f32,
        thumb_h: f32,
        viewport_h: f32,
    },
    /// The point is on a horizontal track but outside the thumb.
    TrackX {
        external_id: ExternalScrollId,
        click_x: f32,
        thumb_x: f32,
        thumb_w: f32,
        viewport_w: f32,
    },
}

/// Given a point in the same coordinate space the scrollbars are drawn
/// in (CSS pixels relative to the iframe reference frame), return the
/// first scrollbar hit, if any. Iterates the scroll tree in the same
/// order as `draw_scrollbars` so the topmost drawn scrollbar wins.
pub(crate) fn hit_test(
    scroll_tree: &ScrollTree,
    point: LayoutPoint,
) -> Option<ScrollbarHit> {
    for node in scroll_tree.nodes.iter() {
        let SpatialTreeNodeInfo::Scroll(scrollable) = &node.info else { continue };

        let viewport = scrollable.clip_rect;
        let vp_w = viewport.size().width;
        let vp_h = viewport.size().height;
        if vp_w < MIN_SCROLLABLE_DIMENSION || vp_h < MIN_SCROLLABLE_DIMENSION {
            continue;
        }

        let content_w = scrollable.content_rect.size().width;
        let content_h = scrollable.content_rect.size().height;
        let scroll_x = scrollable.offset.x;
        let scroll_y = scrollable.offset.y;

        // Vertical scrollbar — right edge column, full height (minus
        // EDGE_INSET so the OS resize handle on the very right edge wins).
        if content_h > vp_h + 0.5 {
            let track_right = viewport.max.x - EDGE_INSET;
            let track_x_min = track_right - THICKNESS;
            if point.x >= track_x_min && point.x <= track_right
                && point.y >= viewport.min.y && point.y <= viewport.max.y
            {
                let raw_thumb = vp_h * (vp_h / content_h);
                let thumb_h = raw_thumb.max(MIN_THUMB_LENGTH).min(vp_h);
                let max_scroll = (content_h - vp_h).max(1.0);
                let max_thumb_y = vp_h - thumb_h;
                let thumb_y = (scroll_y / max_scroll).clamp(0.0, 1.0) * max_thumb_y;
                let thumb_top = viewport.min.y + thumb_y;
                let thumb_bottom = thumb_top + thumb_h;

                if point.y >= thumb_top && point.y <= thumb_bottom {
                    return Some(ScrollbarHit::ThumbY {
                        external_id: scrollable.external_id,
                        thumb_y,
                        thumb_h,
                        viewport_min_y: viewport.min.y,
                        viewport_h: vp_h,
                        content_h,
                    });
                }
                return Some(ScrollbarHit::TrackY {
                    external_id: scrollable.external_id,
                    click_y: point.y,
                    thumb_y: thumb_top,
                    thumb_h,
                    viewport_h: vp_h,
                });
            }
        }

        // Horizontal scrollbar — bottom edge row, accounts for vertical
        // bar AND for the resize handle on the bottom edge.
        if content_w > vp_w + 0.5 {
            let usable_w = vp_w - THICKNESS - EDGE_INSET;
            let track_bottom = viewport.max.y - EDGE_INSET;
            let track_y_min = track_bottom - THICKNESS;
            if point.y >= track_y_min && point.y <= track_bottom
                && point.x >= viewport.min.x && point.x <= viewport.min.x + usable_w
            {
                let raw_thumb = usable_w * (usable_w / content_w);
                let thumb_w = raw_thumb.max(MIN_THUMB_LENGTH).min(usable_w);
                let max_scroll = (content_w - vp_w).max(1.0);
                let max_thumb_x = usable_w - thumb_w;
                let thumb_x = (scroll_x / max_scroll).clamp(0.0, 1.0) * max_thumb_x;
                let thumb_left = viewport.min.x + thumb_x;
                let thumb_right = thumb_left + thumb_w;

                if point.x >= thumb_left && point.x <= thumb_right {
                    return Some(ScrollbarHit::ThumbX {
                        external_id: scrollable.external_id,
                        thumb_x,
                        thumb_w,
                        viewport_min_x: viewport.min.x,
                        viewport_w: vp_w,
                        content_w,
                    });
                }
                return Some(ScrollbarHit::TrackX {
                    external_id: scrollable.external_id,
                    click_x: point.x,
                    thumb_x: thumb_left,
                    thumb_w,
                    viewport_w: vp_w,
                });
            }
        }
    }
    None
}

/// Active drag state: which scrollbar is being dragged, plus the offset
/// from the cursor to the thumb's top/left at drag start. Painter owns
/// one optional instance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DragState {
    pub external_id: ExternalScrollId,
    pub axis: DragAxis,
    /// Cursor's distance from the thumb's start edge at mousedown.
    pub grab_offset: f32,
    /// Cached metrics so we don't refetch from scroll tree each move.
    pub viewport_start: f32, // viewport.min.y for Y, viewport.min.x for X
    pub viewport_size: f32,  // viewport_h for Y, viewport_w for X
    pub content_size: f32,
    pub thumb_size: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DragAxis {
    Y,
    X,
}

/// Given a drag state and current cursor position (in the same coords
/// `hit_test` consumed), compute the new scroll offset for the dragged
/// scrollable. Returns the X or Y offset only — caller composes the
/// full LayoutVector2D using the existing offset for the other axis.
pub(crate) fn compute_drag_offset(state: &DragState, cursor: LayoutPoint) -> f32 {
    let cursor_along = match state.axis {
        DragAxis::Y => cursor.y,
        DragAxis::X => cursor.x,
    };
    let max_thumb_pos = (state.viewport_size - state.thumb_size).max(0.0);
    let thumb_pos = (cursor_along - state.viewport_start - state.grab_offset)
        .clamp(0.0, max_thumb_pos);
    let max_scroll = (state.content_size - state.viewport_size).max(1.0);
    if max_thumb_pos < 0.5 {
        return 0.0;
    }
    (thumb_pos / max_thumb_pos) * max_scroll
}

/// Compose a scroll offset vector for an axis-locked drag, preserving
/// the cross-axis component of the existing offset.
pub(crate) fn drag_offset_to_vector(
    state: &DragState,
    cursor: LayoutPoint,
    existing: LayoutVector2D,
) -> LayoutVector2D {
    let new = compute_drag_offset(state, cursor);
    match state.axis {
        DragAxis::Y => LayoutVector2D::new(existing.x, new),
        DragAxis::X => LayoutVector2D::new(new, existing.y),
    }
}

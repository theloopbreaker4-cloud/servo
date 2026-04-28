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
use webrender_api::units::{LayoutPoint, LayoutRect, LayoutSize};
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
pub(crate) fn draw_scrollbars(
    builder: &mut DisplayListBuilder,
    spatial_id: SpatialId,
    clip_chain_id: ClipChainId,
    scroll_tree: &ScrollTree,
) {
    // Find the largest scroll node — that's the main viewport / document
    // scroll area. Drawing on every scrollable node (which can be 60+ on
    // sites like GitHub that use overflow:scroll heavily) clutters the UI
    // and competes for visual space with the real viewport scrollbar.
    let mut largest: Option<&paint_api::display_list::ScrollableNodeInfo> = None;
    let mut largest_area = 0.0_f32;
    for node in scroll_tree.nodes.iter() {
        let SpatialTreeNodeInfo::Scroll(scrollable) = &node.info else { continue };
        let s = scrollable.clip_rect.size();
        let area = s.width * s.height;
        if area > largest_area {
            largest_area = area;
            largest = Some(scrollable);
        }
    }
    let Some(scrollable) = largest else { return };

    let viewport = scrollable.clip_rect;
    let content_w = scrollable.content_rect.size().width;
    let content_h = scrollable.content_rect.size().height;
    let scroll_x = scrollable.offset.x;
    let scroll_y = scrollable.offset.y;

    // Diagnostic — remove once scrollbar visibility is solid.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("d:/tmp/scrollbar_log.txt")
    {
        use std::io::Write;
        let _ = writeln!(
            f,
            "draw: viewport={}x{} content={}x{} offset=({},{}) draw_y={} draw_x={}",
            viewport.size().width, viewport.size().height,
            content_w, content_h, scroll_x, scroll_y,
            content_h > viewport.size().height + 0.5,
            content_w > viewport.size().width + 0.5,
        );
    }

    draw_axis_y(builder, spatial_id, clip_chain_id, viewport, content_h, scroll_y);
    draw_axis_x(builder, spatial_id, clip_chain_id, viewport, content_w, scroll_x);
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

    let track = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.max.x - THICKNESS, viewport.min.y),
        LayoutSize::new(THICKNESS, viewport_h),
    );
    push_quad(builder, spatial_id, clip_chain_id, track, TRACK_COLOR);

    let raw_thumb = viewport_h * (viewport_h / content_h);
    let thumb_h = raw_thumb.max(MIN_THUMB_LENGTH).min(viewport_h);
    let max_scroll = (content_h - viewport_h).max(1.0);
    let max_thumb_y = viewport_h - thumb_h;
    let thumb_y = (scroll_y / max_scroll).clamp(0.0, 1.0) * max_thumb_y;

    let thumb = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.max.x - THICKNESS, viewport.min.y + thumb_y),
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

    // Reserve space for the vertical scrollbar if it's also visible.
    let usable_w = viewport_w - THICKNESS;

    let track = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.min.x, viewport.max.y - THICKNESS),
        LayoutSize::new(usable_w, THICKNESS),
    );
    push_quad(builder, spatial_id, clip_chain_id, track, TRACK_COLOR);

    let raw_thumb = usable_w * (usable_w / content_w);
    let thumb_w = raw_thumb.max(MIN_THUMB_LENGTH).min(usable_w);
    let max_scroll = (content_w - viewport_w).max(1.0);
    let max_thumb_x = usable_w - thumb_w;
    let thumb_x = (scroll_x / max_scroll).clamp(0.0, 1.0) * max_thumb_x;

    let thumb = LayoutRect::from_origin_and_size(
        LayoutPoint::new(viewport.min.x + thumb_x, viewport.max.y - THICKNESS),
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

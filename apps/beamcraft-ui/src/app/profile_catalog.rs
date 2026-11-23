// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Profile Catalogue dialog (#840): browse model and environment steel section profiles by
//! family and dimensions, inspect a scaled 2D section sketch, select a profile for member creation
//! or element editing, and define new parametric or rectangular profiles via `define-profile`.

use super::App;
use crate::command;
use crate::tools::{Choice, ProfileMeta, ToolError, decode, rejection, text};
use std::path::Path;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Where selecting a profile writes the chosen digest/name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProfileTarget {
    #[default]
    MemberTool,
    Properties,
}

/// Filter by definition location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SourceFilter {
    #[default]
    All,
    InModel,
    Environment,
}

/// Filter by steel section family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FamilyFilter {
    #[default]
    All,
    IShape,
    Channel,
    Angle,
    Rhs,
    Chs,
    ColdFormed,
    Rectangle,
    Custom,
}

impl FamilyFilter {
    const ALL: [(Self, &'static str); 9] = [
        (Self::All, "All families"),
        (Self::IShape, "I / H sections"),
        (Self::Channel, "Channels (U)"),
        (Self::Angle, "Angles (L)"),
        (Self::Rhs, "Rectangular hollow (RHS)"),
        (Self::Chs, "Circular hollow (CHS)"),
        (Self::ColdFormed, "Cold-formed (C/Z/Omega)"),
        (Self::Rectangle, "Rectangles"),
        (Self::Custom, "Custom"),
    ];

    fn matches(self, family: &str) -> bool {
        match self {
            Self::All => true,
            Self::IShape => family == "i_shape",
            Self::Channel => family == "channel",
            Self::Angle => family == "angle",
            Self::Rhs => family == "rhs",
            Self::Chs => family == "chs",
            Self::ColdFormed => matches!(family, "lipped_c" | "plain_c" | "lipped_z" | "omega"),
            Self::Rectangle => family == "rectangle",
            Self::Custom => family == "custom",
        }
    }
}

/// Parametric family options accepted by `define-profile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CreatorFamily {
    #[default]
    IShape,
    Channel,
    Angle,
    Rhs,
    Chs,
    LippedC,
    PlainC,
    LippedZ,
    Omega,
    Rectangle,
}

impl CreatorFamily {
    const ALL: [(Self, &'static str); 10] = [
        (Self::IShape, "I / H section (i_shape)"),
        (Self::Channel, "Channel (channel)"),
        (Self::Angle, "Angle (angle)"),
        (Self::Rhs, "Rectangular hollow (rhs)"),
        (Self::Chs, "Circular hollow (chs)"),
        (Self::LippedC, "Lipped C (lipped_c)"),
        (Self::PlainC, "Plain C (plain_c)"),
        (Self::LippedZ, "Lipped Z (lipped_z)"),
        (Self::Omega, "Omega (omega)"),
        (Self::Rectangle, "Rectangle"),
    ];

    fn token(self) -> &'static str {
        match self {
            Self::IShape => "i_shape",
            Self::Channel => "channel",
            Self::Angle => "angle",
            Self::Rhs => "rhs",
            Self::Chs => "chs",
            Self::LippedC => "lipped_c",
            Self::PlainC => "plain_c",
            Self::LippedZ => "lipped_z",
            Self::Omega => "omega",
            Self::Rectangle => "rectangle",
        }
    }

    pub(crate) fn defaults(self) -> CreatorFields {
        match self {
            Self::IShape => CreatorFields {
                designation: "I-300x150".into(),
                h: "300".into(),
                b: "150".into(),
                tw: "7".into(),
                tf: "10".into(),
                r: "15".into(),
                t: "8".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::Channel => CreatorFields {
                designation: "C-200x75".into(),
                h: "200".into(),
                b: "75".into(),
                tw: "6".into(),
                tf: "9".into(),
                r: "10".into(),
                t: "8".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::Angle => CreatorFields {
                designation: "L-100x100x10".into(),
                h: "100".into(),
                b: "100".into(),
                tw: "7".into(),
                tf: "10".into(),
                r: "10".into(),
                t: "10".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::Rhs => CreatorFields {
                designation: "RHS-200x100x8".into(),
                h: "200".into(),
                b: "100".into(),
                tw: "7".into(),
                tf: "10".into(),
                r: "16".into(),
                t: "8".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::Chs => CreatorFields {
                designation: "CHS-219.1x8".into(),
                h: "219.1".into(),
                b: "219.1".into(),
                tw: "8".into(),
                tf: "8".into(),
                r: "0".into(),
                t: "8".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::LippedC => CreatorFields {
                designation: "LC-200x75x20x2.5".into(),
                h: "200".into(),
                b: "75".into(),
                tw: "2.5".into(),
                tf: "2.5".into(),
                r: "4".into(),
                t: "2.5".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::PlainC => CreatorFields {
                designation: "PC-150x60x3".into(),
                h: "150".into(),
                b: "60".into(),
                tw: "3".into(),
                tf: "3".into(),
                r: "4".into(),
                t: "3".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::LippedZ => CreatorFields {
                designation: "LZ-200x70x20x2.5".into(),
                h: "200".into(),
                b: "70".into(),
                tw: "2.5".into(),
                tf: "2.5".into(),
                r: "4".into(),
                t: "2.5".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
            Self::Omega => CreatorFields {
                designation: "OM-100x50x15x2".into(),
                h: "100".into(),
                b: "50".into(),
                tw: "2".into(),
                tf: "2".into(),
                r: "3".into(),
                t: "2".into(),
                c: "15".into(),
                d: "219.1".into(),
            },
            Self::Rectangle => CreatorFields {
                designation: "Rect-100x200".into(),
                h: "200".into(),
                b: "100".into(),
                tw: "7".into(),
                tf: "10".into(),
                r: "0".into(),
                t: "8".into(),
                c: "20".into(),
                d: "219.1".into(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CreatorFields {
    pub(crate) designation: String,
    pub(crate) h: String,
    pub(crate) b: String,
    pub(crate) tw: String,
    pub(crate) tf: String,
    pub(crate) r: String,
    pub(crate) t: String,
    pub(crate) c: String,
    pub(crate) d: String,
}

impl Default for CreatorFields {
    fn default() -> Self {
        CreatorFamily::IShape.defaults()
    }
}

/// Interactive Profile Catalogue dialog state.
pub(crate) struct State {
    pub(crate) open: bool,
    pub(crate) target: ProfileTarget,
    pub(crate) search: String,
    pub(crate) source_filter: SourceFilter,
    pub(crate) family_filter: FamilyFilter,
    pub(crate) selected_digest: Option<String>,
    pub(crate) creator_open: bool,
    pub(crate) creator_family: CreatorFamily,
    pub(crate) creator: CreatorFields,
    pub(crate) note: Option<Result<String, String>>,
    pub(crate) define_job: Option<Receiver<Result<String, ToolError>>>,
    pub(crate) pending_select_digest: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            target: ProfileTarget::MemberTool,
            search: String::new(),
            source_filter: SourceFilter::All,
            family_filter: FamilyFilter::All,
            selected_digest: None,
            creator_open: false,
            creator_family: CreatorFamily::IShape,
            creator: CreatorFamily::IShape.defaults(),
            note: None,
            define_job: None,
            pending_select_digest: None,
        }
    }
}

impl State {
    pub(crate) fn running(&self) -> bool {
        self.define_job.is_some()
    }
}

fn family_label(family: &str) -> &str {
    match family {
        "i_shape" => "I / H",
        "channel" => "Channel",
        "angle" => "Angle",
        "rhs" => "RHS",
        "chs" => "CHS",
        "lipped_c" => "Lipped C",
        "plain_c" => "Plain C",
        "lipped_z" => "Lipped Z",
        "omega" => "Omega",
        "rectangle" => "Rectangle",
        "custom" => "Custom",
        other => other,
    }
}

fn parse_positive(text: &str, name: &str) -> Result<f64, String> {
    let v = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or_else(|| format!("{name} must be a positive number in mm."))?;
    Ok(v)
}

fn parse_non_negative(text: &str, name: &str) -> Result<f64, String> {
    let v = text
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .ok_or_else(|| format!("{name} must be a non-negative number in mm."))?;
    Ok(v)
}

/// Builds the exact `define-profile` CLI arguments from the creator fields.
pub(crate) fn build_define_profile_args(
    store: &Path,
    family: CreatorFamily,
    fields: &CreatorFields,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        "define-profile".to_owned(),
        "--store".to_owned(),
        store.to_string_lossy().into_owned(),
    ];
    if family == CreatorFamily::Rectangle {
        let width = parse_positive(&fields.b, "Width b")?;
        let height = parse_positive(&fields.h, "Height h")?;
        args.extend([
            "--width".to_owned(),
            width.to_string(),
            "--height".to_owned(),
            height.to_string(),
        ]);
        return Ok(args);
    }
    args.extend(["--family".to_owned(), family.token().to_owned()]);
    match family {
        CreatorFamily::IShape | CreatorFamily::Channel => {
            let h = parse_positive(&fields.h, "h")?;
            let b = parse_positive(&fields.b, "b")?;
            let tw = parse_positive(&fields.tw, "tw")?;
            let tf = parse_positive(&fields.tf, "tf")?;
            let r = parse_non_negative(&fields.r, "r")?;
            args.extend([
                "--h".into(),
                h.to_string(),
                "--b".into(),
                b.to_string(),
                "--tw".into(),
                tw.to_string(),
                "--tf".into(),
                tf.to_string(),
                "--r".into(),
                r.to_string(),
            ]);
        }
        CreatorFamily::Angle | CreatorFamily::Rhs | CreatorFamily::PlainC => {
            let h = parse_positive(&fields.h, "h")?;
            let b = parse_positive(&fields.b, "b")?;
            let t = parse_positive(&fields.t, "t")?;
            let r = parse_non_negative(&fields.r, "r")?;
            args.extend([
                "--h".into(),
                h.to_string(),
                "--b".into(),
                b.to_string(),
                "--t".into(),
                t.to_string(),
                "--r".into(),
                r.to_string(),
            ]);
        }
        CreatorFamily::Chs => {
            let d = parse_positive(&fields.d, "d")?;
            let t = parse_positive(&fields.t, "t")?;
            args.extend(["--d".into(), d.to_string(), "--t".into(), t.to_string()]);
        }
        CreatorFamily::LippedC | CreatorFamily::LippedZ | CreatorFamily::Omega => {
            let h = parse_positive(&fields.h, "h")?;
            let b = parse_positive(&fields.b, "b")?;
            let c = parse_positive(&fields.c, "c")?;
            let t = parse_positive(&fields.t, "t")?;
            let r = parse_non_negative(&fields.r, "r")?;
            args.extend([
                "--h".into(),
                h.to_string(),
                "--b".into(),
                b.to_string(),
                "--c".into(),
                c.to_string(),
                "--t".into(),
                t.to_string(),
                "--r".into(),
                r.to_string(),
            ]);
        }
        CreatorFamily::Rectangle => {}
    }
    let designation = fields.designation.trim();
    if !designation.is_empty() {
        args.extend(["--designation".into(), designation.to_owned()]);
    }
    Ok(args)
}

/// Parses the definition digest returned by `define-profile`.
pub(crate) fn parse_defined_digest(bytes: &[u8]) -> Result<String, ToolError> {
    let reply = decode(bytes)?;
    if let Some(error) = rejection(&reply, Some(0)) {
        return Err(error);
    }
    reply
        .get("definition")
        .ok()
        .and_then(|def| text(def, "digest"))
        .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "define-profile returned no digest"))
}

fn param_value(meta: Option<&ProfileMeta>, key: &str) -> Option<f64> {
    meta?
        .parameters_mm
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| *v)
}

/// Draws a scaled 2D engineering cross-section sketch with `h` / `b` dimension ticks.
fn paint_section_preview(
    ui: &mut egui::Ui,
    choice: &Choice,
    meta: Option<&ProfileMeta>,
    rect: egui::Rect,
) {
    let palette = crate::theme::Palette::current(ui);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, palette.surface);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, palette.rule),
        egui::StrokeKind::Inside,
    );

    let family = meta.map_or("rectangle", |m| m.family.as_str());
    let [raw_w, raw_h] = meta
        .and_then(|m| m.extent_mm)
        .or(choice.rect_mm)
        .unwrap_or([150.0, 300.0]);
    let w_mm = raw_w.max(1.0) as f32;
    let h_mm = raw_h.max(1.0) as f32;

    let pad_l = 32.0;
    let pad_r = 14.0;
    let pad_t = 14.0;
    let pad_b = 26.0;
    let inner = egui::Rect::from_min_max(
        rect.min + egui::vec2(pad_l, pad_t),
        rect.max - egui::vec2(pad_r, pad_b),
    );
    if inner.width() < 12.0 || inner.height() < 12.0 {
        return;
    }
    let scale = (inner.width() / w_mm).min(inner.height() / h_mm).max(0.01);
    let draw_w = w_mm * scale;
    let draw_h = h_mm * scale;
    let box_rect = egui::Rect::from_center_size(inner.center(), egui::vec2(draw_w, draw_h));

    let fill = crate::theme::BRAND.gamma_multiply(0.22);
    let stroke = egui::Stroke::new(1.5, crate::theme::BRAND);

    // Subtle section centre-lines.
    let axis_stroke = egui::Stroke::new(1.0, palette.rule);
    painter.line_segment(
        [
            egui::pos2(box_rect.center().x, box_rect.top() - 4.0),
            egui::pos2(box_rect.center().x, box_rect.bottom() + 4.0),
        ],
        axis_stroke,
    );
    painter.line_segment(
        [
            egui::pos2(box_rect.left() - 4.0, box_rect.center().y),
            egui::pos2(box_rect.right() + 4.0, box_rect.center().y),
        ],
        axis_stroke,
    );

    match family {
        "i_shape" => {
            let tw = (param_value(meta, "tw").unwrap_or((w_mm * 0.06).into()) as f32 * scale)
                .clamp(2.0, draw_w * 0.45);
            let tf = (param_value(meta, "tf").unwrap_or((h_mm * 0.06).into()) as f32 * scale)
                .clamp(2.0, draw_h * 0.35);
            let top = egui::Rect::from_min_max(
                box_rect.left_top(),
                egui::pos2(box_rect.right(), box_rect.top() + tf),
            );
            let bot = egui::Rect::from_min_max(
                egui::pos2(box_rect.left(), box_rect.bottom() - tf),
                box_rect.right_bottom(),
            );
            let web = egui::Rect::from_min_max(
                egui::pos2(box_rect.center().x - tw * 0.5, top.bottom()),
                egui::pos2(box_rect.center().x + tw * 0.5, bot.top()),
            );
            for part in [top, web, bot] {
                painter.rect_filled(part, 0.0, fill);
            }
            let cx = box_rect.center().x;
            let hw = tw * 0.5;
            let outline = vec![
                box_rect.left_top(),
                box_rect.right_top(),
                egui::pos2(box_rect.right(), top.bottom()),
                egui::pos2(cx + hw, top.bottom()),
                egui::pos2(cx + hw, bot.top()),
                egui::pos2(box_rect.right(), bot.top()),
                box_rect.right_bottom(),
                box_rect.left_bottom(),
                egui::pos2(box_rect.left(), bot.top()),
                egui::pos2(cx - hw, bot.top()),
                egui::pos2(cx - hw, top.bottom()),
                egui::pos2(box_rect.left(), top.bottom()),
            ];
            painter.add(egui::Shape::closed_line(outline, stroke));
        }
        "channel" | "plain_c" => {
            let t_def = (w_mm.min(h_mm) * 0.08).into();
            let tw = (param_value(meta, "tw")
                .or_else(|| param_value(meta, "t"))
                .unwrap_or(t_def) as f32
                * scale)
                .clamp(2.0, draw_w * 0.45);
            let tf = (param_value(meta, "tf")
                .or_else(|| param_value(meta, "t"))
                .unwrap_or(t_def) as f32
                * scale)
                .clamp(2.0, draw_h * 0.35);
            let pts = vec![
                box_rect.left_top(),
                box_rect.right_top(),
                egui::pos2(box_rect.right(), box_rect.top() + tf),
                egui::pos2(box_rect.left() + tw, box_rect.top() + tf),
                egui::pos2(box_rect.left() + tw, box_rect.bottom() - tf),
                egui::pos2(box_rect.right(), box_rect.bottom() - tf),
                box_rect.right_bottom(),
                box_rect.left_bottom(),
            ];
            painter.add(egui::Shape::convex_polygon(pts.clone(), fill, stroke));
            // Repaint closed outline cleanly because convex_polygon triangulates concavities.
            painter.rect_filled(box_rect, 0.0, egui::Color32::TRANSPARENT);
            let web = egui::Rect::from_min_max(
                box_rect.left_top(),
                egui::pos2(box_rect.left() + tw, box_rect.bottom()),
            );
            let top = egui::Rect::from_min_max(
                egui::pos2(box_rect.left() + tw, box_rect.top()),
                egui::pos2(box_rect.right(), box_rect.top() + tf),
            );
            let bot = egui::Rect::from_min_max(
                egui::pos2(box_rect.left() + tw, box_rect.bottom() - tf),
                box_rect.right_bottom(),
            );
            for part in [web, top, bot] {
                painter.rect_filled(part, 0.0, fill);
            }
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        "angle" => {
            let t = (param_value(meta, "t").unwrap_or((w_mm.min(h_mm) * 0.1).into()) as f32
                * scale)
                .clamp(2.0, draw_w.min(draw_h) * 0.45);
            let vert = egui::Rect::from_min_max(
                box_rect.left_top(),
                egui::pos2(box_rect.left() + t, box_rect.bottom()),
            );
            let horiz = egui::Rect::from_min_max(
                egui::pos2(box_rect.left() + t, box_rect.bottom() - t),
                box_rect.right_bottom(),
            );
            painter.rect_filled(vert, 0.0, fill);
            painter.rect_filled(horiz, 0.0, fill);
            let outline = vec![
                box_rect.left_top(),
                egui::pos2(box_rect.left() + t, box_rect.top()),
                egui::pos2(box_rect.left() + t, box_rect.bottom() - t),
                egui::pos2(box_rect.right(), box_rect.bottom() - t),
                box_rect.right_bottom(),
                box_rect.left_bottom(),
            ];
            painter.add(egui::Shape::closed_line(outline, stroke));
        }
        "rhs" => {
            let t = (param_value(meta, "t").unwrap_or((w_mm.min(h_mm) * 0.08).into()) as f32
                * scale)
                .clamp(2.0, draw_w.min(draw_h) * 0.35);
            let r = (param_value(meta, "r").unwrap_or(0.0) as f32 * scale)
                .clamp(0.0, draw_w.min(draw_h) * 0.25);
            painter.rect(box_rect, r, fill, stroke, egui::StrokeKind::Middle);
            let hole = box_rect.shrink(t);
            if hole.is_positive() {
                painter.rect(
                    hole,
                    (r - t * 0.6).max(0.0),
                    palette.surface,
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        }
        "chs" => {
            let radius = draw_w.min(draw_h) * 0.5;
            let t = (param_value(meta, "t").unwrap_or((w_mm * 0.08).into()) as f32 * scale)
                .clamp(2.0, radius * 0.7);
            painter.circle(box_rect.center(), radius, fill, stroke);
            if radius - t > 1.0 {
                painter.circle(box_rect.center(), radius - t, palette.surface, stroke);
            }
        }
        "lipped_c" => {
            let t = (param_value(meta, "t").unwrap_or((w_mm.min(h_mm) * 0.05).into()) as f32
                * scale)
                .clamp(2.0, draw_w.min(draw_h) * 0.25);
            let lip = (param_value(meta, "c").unwrap_or((h_mm * 0.14).into()) as f32 * scale)
                .clamp(t + 2.0, draw_h * 0.4);
            let pts = vec![
                box_rect.left_top(),
                box_rect.right_top(),
                egui::pos2(box_rect.right(), box_rect.top() + lip),
                egui::pos2(box_rect.right() - t, box_rect.top() + lip),
                egui::pos2(box_rect.right() - t, box_rect.top() + t),
                egui::pos2(box_rect.left() + t, box_rect.top() + t),
                egui::pos2(box_rect.left() + t, box_rect.bottom() - t),
                egui::pos2(box_rect.right() - t, box_rect.bottom() - t),
                egui::pos2(box_rect.right() - t, box_rect.bottom() - lip),
                egui::pos2(box_rect.right(), box_rect.bottom() - lip),
                box_rect.right_bottom(),
                box_rect.left_bottom(),
            ];
            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        "lipped_z" => {
            let cx = box_rect.center().x;
            let half_b = draw_w * 0.5;
            let lip = (param_value(meta, "c").unwrap_or((h_mm * 0.12).into()) as f32 * scale)
                .clamp(3.0, draw_h * 0.35);
            let z_pts = vec![
                egui::pos2(cx - half_b, box_rect.top() + lip),
                egui::pos2(cx - half_b, box_rect.top()),
                egui::pos2(cx, box_rect.top()),
                egui::pos2(cx, box_rect.bottom()),
                egui::pos2(cx + half_b, box_rect.bottom()),
                egui::pos2(cx + half_b, box_rect.bottom() - lip),
            ];
            painter.add(egui::Shape::line(z_pts, stroke));
        }
        "omega" => {
            let flange = draw_w * 0.22;
            let o_pts = vec![
                box_rect.left_bottom(),
                egui::pos2(box_rect.left() + flange, box_rect.bottom()),
                egui::pos2(box_rect.left() + flange * 1.25, box_rect.top()),
                egui::pos2(box_rect.right() - flange * 1.25, box_rect.top()),
                egui::pos2(box_rect.right() - flange, box_rect.bottom()),
                box_rect.right_bottom(),
            ];
            painter.add(egui::Shape::line(o_pts, stroke));
        }
        _ => {
            painter.rect(box_rect, 0.0, fill, stroke, egui::StrokeKind::Middle);
        }
    }

    // Dimension labels: height on the left, width at the bottom.
    let dim_stroke = egui::Stroke::new(1.0, palette.muted);
    let dim_x = box_rect.left() - 8.0;
    painter.line_segment(
        [
            egui::pos2(dim_x, box_rect.top()),
            egui::pos2(dim_x, box_rect.bottom()),
        ],
        dim_stroke,
    );
    painter.text(
        egui::pos2(dim_x - 3.0, box_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{raw_h:.0}"),
        egui::FontId::proportional(10.0),
        palette.muted,
    );
    let dim_y = box_rect.bottom() + 8.0;
    painter.line_segment(
        [
            egui::pos2(box_rect.left(), dim_y),
            egui::pos2(box_rect.right(), dim_y),
        ],
        dim_stroke,
    );
    painter.text(
        egui::pos2(box_rect.center().x, dim_y + 2.0),
        egui::Align2::CENTER_TOP,
        format!("{raw_w:.0}"),
        egui::FontId::proportional(10.0),
        palette.muted,
    );
}

impl App {
    /// Opens the Profile Catalogue targeted at either the active creation form or Properties.
    pub(crate) fn open_profile_catalog(&mut self, target: ProfileTarget) {
        self.profile_catalog.open = true;
        self.profile_catalog.target = target;
        let active = match target {
            ProfileTarget::MemberTool => self.tools.member.profile.clone(),
            ProfileTarget::Properties => self.props.form.profile.clone(),
        };
        if active.is_some() {
            self.profile_catalog.selected_digest = active;
        } else if self.profile_catalog.selected_digest.is_none() {
            self.profile_catalog.selected_digest = self
                .tools
                .definitions
                .profiles
                .first()
                .map(|c| c.digest.clone());
        }
        if self.tools.definitions.profiles.is_empty() {
            self.profile_catalog.creator_open = true;
        }
    }

    /// Applies the chosen profile `digest` to the caller's target form (`MemberTool` or `Properties`).
    pub(crate) fn apply_profile_selection(&mut self, digest: String) {
        match self.profile_catalog.target {
            ProfileTarget::MemberTool => {
                self.tools.member.profile = Some(digest.clone());
                self.tools.cancel();
            }
            ProfileTarget::Properties => {
                self.props.form.profile = Some(digest.clone());
            }
        }
        self.profile_catalog.selected_digest = Some(digest);
    }

    /// Polls any in-flight `define-profile` command started from the Profile Catalogue.
    pub(crate) fn poll_profile_catalog(&mut self) {
        let Some(rx) = &self.profile_catalog.define_job else {
            return;
        };
        let received = match rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        };
        self.profile_catalog.define_job = None;
        match received {
            Some(Ok(digest)) => {
                self.profile_catalog.pending_select_digest = Some(digest);
                self.profile_catalog.note = Some(Ok("Profile added to the model.".into()));
                self.reload();
            }
            Some(Err(err)) => {
                self.profile_catalog.note = Some(Err(err.to_string()));
            }
            None => {
                self.profile_catalog.note =
                    Some(Err("define-profile worker stopped unexpectedly.".into()));
            }
        }
    }

    fn start_define_profile(&mut self) {
        if self.source.read_only() {
            self.profile_catalog.note =
                Some(Err("Read-only models cannot add new profiles.".into()));
            return;
        }
        if self.profile_catalog.running() || self.other_command_running() {
            self.profile_catalog.note = Some(Err(
                "Another command is running; wait for it to finish.".into(),
            ));
            return;
        }
        let args = match build_define_profile_args(
            &self.source.store,
            self.profile_catalog.creator_family,
            &self.profile_catalog.creator,
        ) {
            Ok(args) => args,
            Err(msg) => {
                self.profile_catalog.note = Some(Err(msg));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = std::sync::Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-define-profile".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|output| parse_defined_digest(&output.stdout));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.profile_catalog.note = None;
                self.profile_catalog.define_job = Some(rx);
            }
            Err(e) => {
                self.profile_catalog.note =
                    Some(Err(format!("Could not start define-profile: {e}")));
            }
        }
    }

    /// Renders the Profile Catalogue floating tool window when `self.profile_catalog.open` is true.
    pub(crate) fn show_profile_catalog(&mut self, ctx: &egui::Context) {
        if !self.profile_catalog.open {
            return;
        }
        let mut open = self.profile_catalog.open;
        let profiles = self.tools.definitions.profiles.clone();
        let meta_map = self.tools.definitions.profile_meta.clone();
        let busy = self.profile_catalog.running() || self.other_command_running();
        let read_only = self.source.read_only();
        let mut apply_and_close: Option<String> = None;
        let mut trigger_define = false;

        crate::kit::tool_window("Profile Catalogue", &mut open, ctx)
            .default_size(egui::vec2(660.0, 500.0))
            .min_size(egui::vec2(520.0, 380.0))
            .show(ctx, |ui| {
                // Top filter toolbar: search + source + target badge.
                ui.horizontal(|ui| {
                    crate::kit::search_field(
                        ui,
                        &mut self.profile_catalog.search,
                        "Search designation or size (e.g. I-300, GOST, 200)",
                        220.0,
                    );
                    egui::ComboBox::from_id_salt("profile-source-filter")
                        .selected_text(match self.profile_catalog.source_filter {
                            SourceFilter::All => "All sources",
                            SourceFilter::InModel => "In model",
                            SourceFilter::Environment => "Environment",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.profile_catalog.source_filter,
                                SourceFilter::All,
                                "All sources",
                            );
                            ui.selectable_value(
                                &mut self.profile_catalog.source_filter,
                                SourceFilter::InModel,
                                "In model",
                            );
                            ui.selectable_value(
                                &mut self.profile_catalog.source_filter,
                                SourceFilter::Environment,
                                "Environment",
                            );
                        });
                    egui::ComboBox::from_id_salt("profile-family-filter")
                        .selected_text(
                            FamilyFilter::ALL
                                .iter()
                                .find(|(f, _)| *f == self.profile_catalog.family_filter)
                                .map_or("All families", |(_, l)| *l),
                        )
                        .show_ui(ui, |ui| {
                            for (variant, label) in FamilyFilter::ALL {
                                ui.selectable_value(
                                    &mut self.profile_catalog.family_filter,
                                    variant,
                                    label,
                                );
                            }
                        });
                });
                ui.add_space(4.0);

                // Filter matches cleanly in display order.
                let needle = self.profile_catalog.search.trim().to_lowercase();
                let filtered: Vec<&Choice> = profiles
                    .iter()
                    .filter(|choice| {
                        let meta = meta_map.get(&choice.digest);
                        let from_env = meta.is_some_and(|m| m.from_environment);
                        let source_ok = match self.profile_catalog.source_filter {
                            SourceFilter::All => true,
                            SourceFilter::InModel => !from_env,
                            SourceFilter::Environment => from_env,
                        };
                        let family = meta.map_or("rectangle", |m| m.family.as_str());
                        let family_ok = self.profile_catalog.family_filter.matches(family);
                        let search_ok = needle.is_empty()
                            || choice.label.to_lowercase().contains(&needle)
                            || meta.is_some_and(|m| {
                                m.designation.to_lowercase().contains(&needle)
                                    || m.standard
                                        .as_deref()
                                        .is_some_and(|s| s.to_lowercase().contains(&needle))
                                    || m.family.to_lowercase().contains(&needle)
                            });
                        source_ok && family_ok && search_ok
                    })
                    .collect();

                let palette = crate::theme::Palette::current(ui);
                let avail_h = (ui.available_height() - 120.0).max(160.0);

                // Split body: scrollable catalogue list on the left, 2D section sketch & specs on the right.
                ui.horizontal_top(|ui| {
                    let list_w = (ui.available_width() * 0.56).max(260.0);
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        ui.label(
                            egui::RichText::new(format!("{} profile(s)", filtered.len()))
                                .size(11.0)
                                .color(palette.muted),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("profile-catalog-list")
                            .max_height(avail_h)
                            .show(ui, |ui| {
                                if filtered.is_empty() {
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(
                                            "No profiles match or catalogue is empty. Create a parametric profile below.",
                                        )
                                        .color(palette.muted),
                                    );
                                }
                                for choice in &filtered {
                                    let meta = meta_map.get(&choice.digest);
                                    let is_selected = self
                                        .profile_catalog
                                        .selected_digest
                                        .as_deref()
                                        == Some(choice.digest.as_str());
                                    let tag = if meta.is_some_and(|m| m.from_environment) {
                                        "Env"
                                    } else {
                                        "Model"
                                    };
                                    let fam = meta.map_or("Rect", |m| family_label(&m.family));
                                    let title = meta
                                        .map(|m| m.designation.as_str())
                                        .unwrap_or(choice.label.as_str());
                                    let resp = ui.selectable_label(
                                        is_selected,
                                        format!("[{tag}] {title}  ·  {fam}"),
                                    );
                                    if resp.clicked() {
                                        self.profile_catalog.selected_digest =
                                            Some(choice.digest.clone());
                                    }
                                    if resp.double_clicked() {
                                        apply_and_close = Some(choice.digest.clone());
                                    }
                                }
                            });
                    });

                    ui.separator();

                    // Right engineering inspector: 2D cross-section drawing + table of parameters.
                    ui.vertical(|ui| {
                        let selected = self.profile_catalog.selected_digest.as_deref().and_then(
                            |digest| profiles.iter().find(|c| c.digest == digest),
                        );
                        if let Some(choice) = selected {
                            let meta = meta_map.get(&choice.digest);
                            let (sketch_rect, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width().max(160.0), 130.0),
                                egui::Sense::hover(),
                            );
                            paint_section_preview(ui, choice, meta, sketch_rect);
                            ui.add_space(6.0);
                            if let Some(m) = meta {
                                ui.strong(&m.designation);
                                crate::kit::prop_info(
                                    ui,
                                    "Family",
                                    Some(family_label(&m.family)),
                                    "—",
                                );
                                crate::kit::prop_info(
                                    ui,
                                    "Source",
                                    Some(if m.from_environment {
                                        "Environment (auto-pins on use)"
                                    } else {
                                        "Pinned in model"
                                    }),
                                    "—",
                                );
                                if let Some(std_name) = &m.standard {
                                    crate::kit::prop_info(ui, "Standard", Some(std_name), "—");
                                }
                                if !m.parameters_mm.is_empty() {
                                    let summary = m
                                        .parameters_mm
                                        .iter()
                                        .map(|(k, v)| format!("{k}={v}"))
                                        .collect::<Vec<_>>()
                                        .join("  ");
                                    crate::kit::prop_info(ui, "Dims (mm)", Some(&summary), "—");
                                }
                                if let Some(area) = m.area_mm2 {
                                    crate::kit::prop_info(
                                        ui,
                                        "Area",
                                        Some(&format!("{area:.1} mm²")),
                                        "—",
                                    );
                                }
                                if let Some(mass) = m.mass_per_meter_kg {
                                    crate::kit::prop_info(
                                        ui,
                                        "Mass",
                                        Some(&format!("{mass:.2} kg/m")),
                                        "—",
                                    );
                                }
                            } else {
                                ui.strong(&choice.label);
                            }
                        } else {
                            ui.label(
                                egui::RichText::new("Select a profile on the left to inspect.")
                                    .color(palette.muted),
                            );
                        }
                    });
                });

                ui.add_space(4.0);
                // Expandable parametric profile creator (`define-profile`).
                let header = egui::CollapsingHeader::new("+ Create parametric profile in model")
                    .default_open(self.profile_catalog.creator_open || profiles.is_empty())
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!busy && !read_only, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Family");
                                let before = self.profile_catalog.creator_family;
                                egui::ComboBox::from_id_salt("creator-profile-family")
                                    .selected_text(
                                        CreatorFamily::ALL
                                            .iter()
                                            .find(|(f, _)| *f == before)
                                            .map_or("I / H", |(_, l)| *l),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (variant, label) in CreatorFamily::ALL {
                                            ui.selectable_value(
                                                &mut self.profile_catalog.creator_family,
                                                variant,
                                                label,
                                            );
                                        }
                                    });
                                if self.profile_catalog.creator_family != before {
                                    self.profile_catalog.creator =
                                        self.profile_catalog.creator_family.defaults();
                                }
                                if self.profile_catalog.creator_family != CreatorFamily::Rectangle {
                                    ui.label("Name");
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.profile_catalog.creator.designation,
                                        )
                                        .desired_width(120.0),
                                    );
                                }
                            });

                            let fam = self.profile_catalog.creator_family;
                            let f = &mut self.profile_catalog.creator;
                            ui.horizontal_wrapped(|ui| {
                                let mut dim = |label: &str, val: &mut String| {
                                    ui.label(label);
                                    ui.add(egui::TextEdit::singleline(val).desired_width(48.0));
                                };
                                match fam {
                                    CreatorFamily::Rectangle => {
                                        dim("b", &mut f.b);
                                        dim("h", &mut f.h);
                                    }
                                    CreatorFamily::IShape | CreatorFamily::Channel => {
                                        dim("h", &mut f.h);
                                        dim("b", &mut f.b);
                                        dim("tw", &mut f.tw);
                                        dim("tf", &mut f.tf);
                                        dim("r", &mut f.r);
                                    }
                                    CreatorFamily::Angle
                                    | CreatorFamily::Rhs
                                    | CreatorFamily::PlainC => {
                                        dim("h", &mut f.h);
                                        dim("b", &mut f.b);
                                        dim("t", &mut f.t);
                                        dim("r", &mut f.r);
                                    }
                                    CreatorFamily::Chs => {
                                        dim("d", &mut f.d);
                                        dim("t", &mut f.t);
                                    }
                                    CreatorFamily::LippedC
                                    | CreatorFamily::LippedZ
                                    | CreatorFamily::Omega => {
                                        dim("h", &mut f.h);
                                        dim("b", &mut f.b);
                                        dim("c", &mut f.c);
                                        dim("t", &mut f.t);
                                        dim("r", &mut f.r);
                                    }
                                }
                                if crate::theme::button(
                                    ui,
                                    crate::theme::Kind::Secondary,
                                    "Add to model",
                                    !busy && !read_only,
                                )
                                .clicked()
                                {
                                    trigger_define = true;
                                }
                            });
                        });
                    });
                self.profile_catalog.creator_open = header.openness > 0.0;

                if let Some(note) = &self.profile_catalog.note {
                    let (msg, color) = match note {
                        Ok(s) => (s.as_str(), palette.text),
                        Err(s) => (s.as_str(), palette.error),
                    };
                    ui.colored_label(color, msg);
                }

                // Unified bottom action bar.
                let can_select = self.profile_catalog.selected_digest.is_some();
                let done =
                    crate::kit::actions(ui, "Select profile", can_select, "Close", busy);
                if done.primary
                    && let Some(digest) = self.profile_catalog.selected_digest.clone()
                {
                    apply_and_close = Some(digest);
                }
                if done.cancel {
                    self.profile_catalog.open = false;
                }
            });

        self.profile_catalog.open = self.profile_catalog.open && open;
        if trigger_define {
            self.start_define_profile();
        }
        if let Some(digest) = apply_and_close {
            self.apply_profile_selection(digest);
            self.profile_catalog.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_define_profile_args_matches_cli_contract_for_all_families() {
        let store = Path::new("model");
        let rect = build_define_profile_args(
            store,
            CreatorFamily::Rectangle,
            &CreatorFamily::Rectangle.defaults(),
        )
        .unwrap();
        assert_eq!(
            rect,
            [
                "define-profile",
                "--store",
                "model",
                "--width",
                "100",
                "--height",
                "200"
            ]
        );

        let isec = build_define_profile_args(
            store,
            CreatorFamily::IShape,
            &CreatorFamily::IShape.defaults(),
        )
        .unwrap();
        assert!(isec.windows(2).any(|w| w == ["--family", "i_shape"]));
        assert!(isec.windows(2).any(|w| w == ["--h", "300"]));
        assert!(isec.windows(2).any(|w| w == ["--designation", "I-300x150"]));

        let chs =
            build_define_profile_args(store, CreatorFamily::Chs, &CreatorFamily::Chs.defaults())
                .unwrap();
        assert!(chs.windows(2).any(|w| w == ["--family", "chs"]));
        assert!(chs.windows(2).any(|w| w == ["--d", "219.1"]));
        assert!(chs.windows(2).any(|w| w == ["--t", "8"]));
    }

    #[test]
    fn parse_defined_digest_reads_definition_digest_or_rejection() {
        let ok = br#"{"outcome":"definition_created","definition":{"digest":"abc123"}}"#;
        assert_eq!(parse_defined_digest(ok).unwrap(), "abc123");

        let rej = br#"{"outcome":"rejected","code":"INVALID_ARGUMENT","message":"bad size"}"#;
        assert!(parse_defined_digest(rej).is_err());
    }
}

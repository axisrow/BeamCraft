// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The sheet editor against a real document and the real CLI: applying placements puts every
//! view kind and title back, and the preview draws the export's own title block, so a marker
//! the preview shows inside the block is a marker the export refuses. The gestures follow the
//! `contour_grips` idiom: the editor's own command builder runs against the real CLI, and the
//! real widget render pins the preview to the export.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use crate::support;
use beamcraft_json::Json;
use beamcraft_ui::sheet::{SheetEditor, apply_args, parse_doc};
use beamcraft_ui::stats::Stats;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

const VIEW: &str = "01900000-0000-7000-8000-000000000401";
const MEMBER: &str = "01900000-0000-7000-8000-000000000402";
const DRAWING: &str = "01900000-0000-7000-8000-000000000403";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000404";
const PLAN: &str = "01900000-0000-7000-8000-000000000405";
const SECTION: &str = "01900000-0000-7000-8000-000000000406";
const DETAIL: &str = "01900000-0000-7000-8000-000000000407";

/// The marker colour the editor paints movable views with.
const MARKER: egui::Color32 = egui::Color32::from_rgb(20, 90, 200);

/// Runs the CLI next to this package's binary and decodes its JSON reply.
fn run(cli: &Path, args: &[&str]) -> Json {
    let output = Command::new(cli).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap()
}

/// A document with an a3 sheet, a titled primary view and one placed view, section and
/// detail, all titled; returns the document directory. `plan_origin` is the plan's paper
/// origin in millimetres.
fn document(cli: &Path, root: &Path, plan_origin: &str) -> String {
    let project = run(
        cli,
        &[
            "create-project",
            "--parent",
            root.to_str().unwrap(),
            "--name",
            "m",
        ],
    );
    let store = project
        .get("project")
        .unwrap()
        .get("path")
        .unwrap()
        .string()
        .unwrap()
        .to_owned();
    let profile = run(
        cli,
        &[
            "define-profile",
            "--store",
            &store,
            "--width",
            "100",
            "--height",
            "50",
        ],
    );
    let digest = profile
        .get("definition")
        .unwrap()
        .get("digest")
        .unwrap()
        .string()
        .unwrap()
        .to_owned();
    run(
        cli,
        &[
            "create-member",
            "--store",
            &store,
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &digest,
            "--new-id",
            MEMBER,
        ],
    );
    let doc = root.join("doc");
    let doc_str = doc.to_str().unwrap().to_owned();
    let anchor_a = format!("fid1:{MEMBER}:member:v.x0.y0.z0");
    let anchor_b = format!("fid1:{MEMBER}:member:v.x0.y0.z1");
    let created = run(
        cli,
        &[
            "create-drawing",
            "--store",
            &store,
            "--document-dir",
            &doc_str,
            "--op-id",
            "01900000-0000-7000-8000-000000000411",
            "--revision-id",
            "01900000-0000-7000-8000-000000000412",
            "--actor-id",
            "01900000-0000-7000-8000-000000000413",
            "--id",
            MEMBER,
            "--drawing-id",
            DRAWING,
            "--view-id",
            VIEW,
            "--dimension-id",
            DIMENSION,
            "--anchor-a",
            &anchor_a,
            "--anchor-b",
            &anchor_b,
            "--origin",
            "30,100",
            "--scale",
            "20",
            "--view-title",
            "FRONT",
            "--sheet-size",
            "a3",
            "--sheet-project",
            "P",
            "--sheet-author",
            "A",
        ],
    );
    let head = created
        .get("revision_id")
        .unwrap()
        .string()
        .unwrap()
        .to_owned();
    let plan_spec = format!("{PLAN}:top:20:{plan_origin}:PLAN");
    let section_spec = format!("{SECTION}:right:1500:positive:5:230,250:SECTION A-A");
    let detail_spec = format!("{DETAIL}:{VIEW}:30,100:4:5:230,330:DETAIL B");
    run(
        cli,
        &[
            "edit-views",
            "--document-dir",
            &doc_str,
            "--expected-head",
            &head,
            "--op-id",
            "01900000-0000-7000-8000-000000000414",
            "--revision-id",
            "01900000-0000-7000-8000-000000000415",
            "--actor-id",
            "01900000-0000-7000-8000-000000000416",
            "--placed-view",
            &plan_spec,
            "--placed-section",
            &section_spec,
            "--placed-detail",
            &detail_spec,
        ],
    );
    doc_str
}

/// The egui context of the window: the installed theme and the real Inter faces.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    beamcraft_ui::theme::install(&ctx);
    let mut fonts = egui::FontDefinitions::empty();
    let inter = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/inter/Inter-Regular.ttf"),
    )
    .unwrap();
    fonts
        .font_data
        .insert("Inter".into(), egui::FontData::from_owned(inter).into());
    for family in [
        egui::FontFamily::Proportional,
        egui::FontFamily::Monospace,
        beamcraft_ui::theme::bold_family(),
    ] {
        fonts.families.insert(family, vec!["Inter".into()]);
    }
    ctx.set_fonts(fonts);
    ctx
}

/// The centre of one rendered text galley, matched exactly by its text.
fn text_at(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.text() == label
            {
                Some(text.pos + text.galley.size() / 2.0)
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("no {label} rendered"))
}

/// The centre of the first text galley that contains the given text, if any.
fn text_contains(output: &egui::FullOutput, needle: &str) -> Option<egui::Pos2> {
    output.shapes.iter().find_map(|shape| {
        if let egui::Shape::Text(text) = &shape.shape
            && text.galley.text().contains(needle)
        {
            Some(text.pos + text.galley.size() / 2.0)
        } else {
            None
        }
    })
}

/// Every rendered text galley, for failure reports.
fn texts(output: &egui::FullOutput) -> Vec<String> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                Some(text.galley.text().to_owned())
            } else {
                None
            }
        })
        .collect()
}

/// The centres of the movable markers, in draw order: views, then sections, then details.
fn markers(output: &egui::FullOutput) -> Vec<egui::Pos2> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Circle(circle) = &shape.shape
                && circle.fill == MARKER
            {
                Some(circle.center)
            } else {
                None
            }
        })
        .collect()
}

/// The filled paper rectangle of the preview.
fn paper_rect(output: &egui::FullOutput) -> egui::Rect {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape {
                Some((rect.fill, rect.rect))
            } else {
                None
            }
        })
        .find(|(fill, _)| *fill == egui::Color32::from_gray(250))
        .map(|(_, rect)| rect)
        .unwrap_or_else(|| {
            panic!(
                "no paper rectangle rendered; rect fills: {:?}",
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Rect(rect) = &shape.shape {
                            Some((rect.fill, rect.stroke, rect.rect))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            )
        })
}

/// The stroked rectangle outlines of the preview, black only.
fn stroked_rects(output: &egui::FullOutput) -> Vec<egui::Rect> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.stroke.color == egui::Color32::BLACK
            {
                Some(rect.rect)
            } else {
                None
            }
        })
        .collect()
}

/// One frame of the sheet editor window with the given pointer events.
fn frame(
    ctx: &egui::Context,
    editor: &mut SheetEditor,
    cli: &Path,
    stats: &Arc<Stats>,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(2000.0, 2000.0),
        )),
        events,
        ..Default::default()
    };
    let editor = &mut *editor;
    ctx.run_ui(input, move |_ui| editor.window(ctx, cli, None, stats))
}

/// The current pointer event stream: nothing but a hover at the given position.
fn hover(pos: egui::Pos2) -> Vec<egui::Event> {
    vec![egui::Event::PointerMoved(pos)]
}

/// One idle frame at the given pointer position, keeping the editor's background jobs moving.
fn idle(
    ctx: &egui::Context,
    editor: &mut SheetEditor,
    cli: &Path,
    stats: &Arc<Stats>,
    pos: egui::Pos2,
) -> egui::FullOutput {
    frame(ctx, editor, cli, stats, hover(pos))
}

/// Frames the editor until the given text renders, keeping background jobs moving.
fn settle_until_text(
    ctx: &egui::Context,
    editor: &mut SheetEditor,
    cli: &Path,
    stats: &Arc<Stats>,
    needle: &str,
    timeout: Duration,
) -> egui::FullOutput {
    let deadline = Instant::now() + timeout;
    let mut output = idle(ctx, editor, cli, stats, egui::Pos2::ZERO);
    loop {
        if text_contains(&output, needle).is_some() {
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "the editor never rendered {needle:?}; texts rendered: {:?}",
            texts(&output)
        );
        output.drop_without_applying_deltas();
        std::thread::sleep(Duration::from_millis(25));
        output = idle(ctx, editor, cli, stats, egui::Pos2::ZERO);
    }
}

/// The click gesture at the given position. The pointer parks on the position for two frames
/// first: the hit-test reads the pointer with a frame of delay, so an early press lands
/// nowhere.
fn click(
    ctx: &egui::Context,
    editor: &mut SheetEditor,
    cli: &Path,
    stats: &Arc<Stats>,
    pos: egui::Pos2,
) {
    for _ in 0..2 {
        frame(ctx, editor, cli, stats, hover(pos)).drop_without_applying_deltas();
    }
    frame(
        ctx,
        editor,
        cli,
        stats,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
    frame(ctx, editor, cli, stats, hover(pos)).drop_without_applying_deltas();
    frame(
        ctx,
        editor,
        cli,
        stats,
        vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    )
    .drop_without_applying_deltas();
}

/// Renders tessellated shapes into a PNG at one pixel per point, for the wave evidence.
/// `textures` carries the deltas of the frames this context produced, the font atlas
/// included: every mesh references the atlas texture, so an unapplied delta paints nothing.
/// The capture only runs when `BEAMCRAFT_SHEET_PREVIEW_PNG_DIR` names an existing directory.
fn render_png(
    primitives: Vec<egui::epaint::ClippedPrimitive>,
    textures: &[(egui::TextureId, Vec<egui::epaint::ImageDelta>)],
    path: &Path,
) {
    use eframe::egui_wgpu;
    let size = [2000u32, 2000];
    let gpu = pollster::block_on(beamcraft_ui::render::Gpu::headless()).unwrap();
    let descriptor = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: 1.0,
    };
    let mut painter = egui_wgpu::Renderer::new(
        &gpu.device,
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sheet preview evidence"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sheet preview evidence"),
        });
    for (id, steps) in textures {
        for delta in steps {
            painter.update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
    }
    let mut buffers = painter.update_buffers(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &primitives,
        &descriptor,
    );
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sheet preview evidence"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        painter.render(&mut pass, &primitives, &descriptor);
    }
    buffers.push(encoder.finish());
    let rendered = gpu.queue.submit(buffers);
    let row = (size[0] * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sheet preview evidence"),
        size: u64::from(row) * u64::from(size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("sheet preview readback"),
        });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    let index = gpu.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = tx.send(result);
    });
    beamcraft_ui::render::wait(&gpu.device, index).unwrap();
    beamcraft_ui::render::wait(&gpu.device, rendered).unwrap();
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the readback never mapped")
        .expect("the readback failed");
    let mapped = buffer
        .get_mapped_range(..)
        .expect("the readback is unmapped");
    let data = mapped.to_vec();
    drop(mapped);
    buffer.unmap();
    let mut tight = Vec::with_capacity(
        usize::try_from(size[0]).unwrap() * 4 * usize::try_from(size[1]).unwrap(),
    );
    for y in 0..size[1] {
        let start = (y as usize) * (row as usize);
        let end = start + (size[0] as usize) * 4;
        tight.extend_from_slice(&data[start..end]);
    }
    let image = image::RgbaImage::from_raw(size[0], size[1], tight).unwrap();
    image
        .save(path)
        .expect("the evidence PNG could not be written");
}

/// The editor loads a real document through the real `inspect-drawing`, and its own command
/// builder puts every placed family back through the real `edit-views`: the apply of one
/// moved origin survives the section, the detail and all three titles.
#[test]
fn apply_args_sends_every_view_kind_back_through_the_real_cli() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sheet-editor")
        .join(format!("apply-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let document = document(&cli, &root, "30,250");
    // The document mirror the editor itself builds from the real inspect reply.
    let reply = run(&cli, &["inspect-drawing", "--document-dir", &document]);
    let doc = parse_doc(&reply).expect("the sheet editor mirror reads the real reply");
    assert_eq!(doc.placed.len(), 1, "the plan");
    assert_eq!(doc.sections.len(), 1, "the section");
    assert_eq!(doc.details.len(), 1, "the detail");
    assert_eq!(doc.placed[0].title.as_deref(), Some("PLAN"));
    assert_eq!(doc.sections[0].title.as_deref(), Some("SECTION A-A"));
    assert_eq!(doc.details[0].title.as_deref(), Some("DETAIL B"));
    // The drag changed only the plan's pending origin; the apply must put every family back,
    // moved or not, or edit-views deletes the families it was not given.
    let mut pending = vec![
        doc.placed[0].origin,
        doc.sections[0].origin,
        doc.details[0].origin,
    ];
    pending[0][0] += 10.0;
    let args = apply_args(
        &document,
        &doc.revision_id,
        "01900000-0000-7000-8000-000000000421",
        "01900000-0000-7000-8000-000000000422",
        "01900000-0000-7000-8000-000000000423",
        &doc,
        &pending,
    );
    assert_eq!(
        args.iter()
            .filter(|arg| arg.as_str() == "--placed-view")
            .count(),
        1,
        "one placed view"
    );
    assert_eq!(
        args.iter()
            .filter(|arg| arg.as_str() == "--placed-section")
            .count(),
        1,
        "one placed section rides along"
    );
    assert_eq!(
        args.iter()
            .filter(|arg| arg.as_str() == "--placed-detail")
            .count(),
        1,
        "one placed detail rides along"
    );
    let output = Command::new(&cli).args(&args).output().unwrap();
    assert!(
        output.status.success(),
        "{:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let applied = run(&cli, &["inspect-drawing", "--document-dir", &document]);
    let views = applied.get("placed_views").unwrap().array().unwrap();
    let sections = applied.get("placed_sections").unwrap().array().unwrap();
    let details = applied.get("placed_details").unwrap().array().unwrap();
    assert_eq!(views.len(), 1, "the placed view must survive the apply");
    assert_eq!(sections.len(), 1, "the section must survive the apply");
    assert_eq!(details.len(), 1, "the detail must survive the apply");
    assert_eq!(
        views[0].get("title").unwrap().string().unwrap(),
        "PLAN",
        "the placed title must survive the apply"
    );
    assert_eq!(
        sections[0].get("title").unwrap().string().unwrap(),
        "SECTION A-A",
        "the section title must survive the apply"
    );
    assert_eq!(
        details[0].get("title").unwrap().string().unwrap(),
        "DETAIL B",
        "the detail title must survive the apply"
    );
    let origin: Vec<f64> = views[0]
        .get("paper_origin_mm")
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|value| value.float().unwrap())
        .collect();
    assert!(
        (origin[0] - 40.0).abs() < 0.01,
        "the moved origin was applied: {origin:?}"
    );
    assert!(
        (origin[1] - 250.0).abs() < 0.01,
        "the untouched origin coordinate passes through: {origin:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The preview draws the engine's title block: the same block the export draws and enforces,
/// so the issue's plan at 135,35 — which the export refuses as overlapping — sits inside the
/// drawn block in the editor too, next to its section and detail markers.
#[test]
fn the_preview_draws_the_export_title_block() {
    let Some(cli) = support::cli(Path::new(env!("CARGO_BIN_EXE_beamcraft-ui"))) else {
        panic!("the beamcraft CLI is required next to the beamcraft-ui binary");
    };
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sheet-editor")
        .join(format!("block-{}", support::boot_nonce()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let document = document(&cli, &root, "135,35");
    let ctx = context();
    let stats = Arc::new(Stats::default());
    let mut editor = SheetEditor::default();
    editor.open = true;
    editor.document_dir = document.clone();
    // The first frame lays the fonts out: its texture delta carries the atlas every later
    // mesh paints with, so it is kept for the evidence render; the second paints with them.
    let mut textures: Vec<(egui::TextureId, Vec<egui::epaint::ImageDelta>)> = Vec::new();
    let fonts = idle(&ctx, &mut editor, &cli, &stats, egui::Pos2::ZERO);
    for (id, steps) in &fonts.textures_delta.set {
        textures.push((*id, steps.to_vec()));
    }
    fonts.drop_without_applying_deltas();
    let first = idle(&ctx, &mut editor, &cli, &stats, egui::Pos2::ZERO);
    let load = text_at(&first, "Load");
    first.drop_without_applying_deltas();
    click(&ctx, &mut editor, &cli, &stats, load);
    let output = settle_until_text(
        &ctx,
        &mut editor,
        &cli,
        &stats,
        "sections and",
        Duration::from_secs(30),
    );
    // The settled frame grew the auto-sized window; one more idle paints at the final layout.
    output.drop_without_applying_deltas();
    let mut output = idle(&ctx, &mut editor, &cli, &stats, egui::Pos2::ZERO);
    let paper = paper_rect(&output);
    let scale = paper.width() / 297.0;
    let rects = stroked_rects(&output);
    let marker = markers(&output);
    // The wave evidence: a PNG of the whole editor window, one frame of the real widget.
    if let Ok(dir) = std::env::var("BEAMCRAFT_SHEET_PREVIEW_PNG_DIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            let primitives = ctx.tessellate(output.shapes.clone(), 1.0);
            render_png(primitives, &textures, &dir.join("sheet-preview.png"));
        }
    }
    output.textures_delta.clear();
    // The frame outline is the black stroke whose width matches the sheet interior; the
    // title block is the other black stroke, anchored at its bottom-right corner.
    let frame = rects
        .iter()
        .copied()
        .find(|rect| (rect.width() - (297.0 - 20.0) * scale).abs() < 1.0)
        .expect("the frame outline is not rendered");
    let block = rects
        .iter()
        .copied()
        .find(|rect| {
            *rect != frame && rect.right() == frame.right() && rect.bottom() == frame.bottom()
        })
        .expect("the title block is not rendered");
    let expected = beamcraft_engine::drawing::sheet::TITLE_BLOCK_WIDTH_MM as f32 * scale;
    assert!(
        (block.width() - expected).abs() < 0.5,
        "the preview block is {} mm wide, the export block is {expected} mm",
        block.width() / scale
    );
    assert_eq!(marker.len(), 3, "one marker per view, section and detail");
    assert!(
        block.contains(marker[0]),
        "the plan marker at 135,35 sits outside the preview block although the export refuses it as overlapping"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 named views: a bounded registry of saved view poses plus the plane views that derive a
//! pose from a grid line, a level or two points. A view is display state only: nothing here
//! reads or writes a store, so saving or applying a view can never change the model. The JSON
//! session format lives in `beamcraft_host::views`, the one layer both the CLI and the window
//! already share.

use crate::grid_query;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::grid::Grid;
use beamcraft_model::limits::MAX_LABEL_BYTES;
use std::collections::BTreeMap;

/// The orbit camera's target bound (metres of steel): a pose outside it is refused, so a
/// restored view never leaves what the window's camera accepts.
const TARGET_CAP_MM: f64 = 1e7;
/// Distance bounds, the orbit camera's own.
const MIN_DISTANCE_MM: f64 = 10.0;
const MAX_DISTANCE_MM: f64 = 2e7;
/// The steepest admissible pitch: just short of the nadir, where the orbit basis would fold.
pub const PITCH_MAX_RAD: f64 = 1.52;
/// Saved views per model.
pub const MAX_VIEWS: usize = 128;
/// A view name is a short label.
const MAX_NAME_BYTES: usize = 64;

fn bad(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

/// Where a saved camera stood: target, orbit angles, distance and projection, in millimetres
/// and radians. Mirrors the window's orbit camera bounds, so a restore is exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewPose {
    /// Orbit target in model millimetres.
    pub target_mm: [f64; 3],
    /// Orbit yaw in radians.
    pub yaw: f64,
    /// Orbit pitch in radians, `0.0..=[PITCH_MAX_RAD]`.
    pub pitch: f64,
    /// Orbit distance in millimetres.
    pub distance_mm: f64,
    /// Orthographic projection when true, perspective otherwise.
    pub orthographic: bool,
}

impl ViewPose {
    /// Admits a bounded pose: finite angles, capped target, camera distance bounds.
    pub fn new(
        target_mm: [f64; 3],
        yaw: f64,
        pitch: f64,
        distance_mm: f64,
        orthographic: bool,
    ) -> Result<Self, CommandError> {
        if !target_mm.iter().all(|v| v.is_finite())
            || !yaw.is_finite()
            || !pitch.is_finite()
            || !distance_mm.is_finite()
        {
            return Err(bad(
                ErrorCode::NonFiniteNumber,
                "views.pose",
                "pose value is not finite",
            ));
        }
        if target_mm.iter().any(|v| v.abs() > TARGET_CAP_MM) {
            return Err(bad(
                ErrorCode::LimitExceeded,
                "views.pose",
                "pose target exceeds the coordinate cap",
            ));
        }
        if !(MIN_DISTANCE_MM..=MAX_DISTANCE_MM).contains(&distance_mm) {
            return Err(bad(
                ErrorCode::LimitExceeded,
                "views.pose",
                "pose distance is outside the camera bounds",
            ));
        }
        if !(0.0..=PITCH_MAX_RAD).contains(&pitch) {
            return Err(bad(
                ErrorCode::InvalidArgument,
                "views.pose",
                "pose pitch is outside the orbit range",
            ));
        }
        Ok(Self {
            target_mm,
            yaw: yaw.rem_euclid(std::f64::consts::TAU),
            pitch,
            distance_mm,
            orthographic,
        })
    }
}

/// A plane view defined by geometry the model already names. The pose is resolved once at save
/// time, so applying the saved view needs no store.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaneSpec {
    /// A vertical view plane on a grid axis: the eye looks along the plane's horizontal normal,
    /// target at the line's world midpoint. `reverse` looks from the other side.
    GridLine {
        /// Grid selector: canonical id or unambiguous exact name.
        grid: String,
        /// Axis label within that grid.
        line: String,
        /// Look from the opposite side of the plane.
        reverse: bool,
    },
    /// A near-top-down plan view centred on the frame centre. The orbit basis has no exact
    /// nadir, so the pitch is [`PITCH_MAX_RAD`] and the yaw is `0`.
    Level,
    /// Look from the first point toward the second; the target is the second point. A view from
    /// below flattens to horizontal, because the orbit pitch never goes negative.
    TwoPoints {
        /// Eye side point, model millimetres.
        from_mm: [f64; 3],
        /// Target point, model millimetres.
        to_mm: [f64; 3],
    },
}

/// The axis-aligned model box a view works inside, in millimetres. One type serves V12 and the
/// cut plane: V15's [`crate::view_clipping::WorkArea`] is the admission and clipping source of
/// truth, and saved views carry it.
pub use crate::view_clipping::WorkArea;

/// One saved view: its name, the plane it came from (`None` for a free 3D view), the pose that
/// restores it and the work area it carried.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedView {
    /// Non-empty trimmed label, unique in the book.
    pub name: String,
    /// The plane definition, when this is a plane view.
    pub plane: Option<PlaneSpec>,
    /// The pose that restores this view.
    pub pose: ViewPose,
    /// The work area this view carries, when one was set.
    pub work_area: Option<WorkArea>,
}

impl NamedView {
    /// Admits a named view: the name is trimmed non-empty and short.
    pub fn new(
        name: &str,
        plane: Option<PlaneSpec>,
        pose: ViewPose,
        work_area: Option<WorkArea>,
    ) -> Result<Self, CommandError> {
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_NAME_BYTES.min(MAX_LABEL_BYTES) {
            return Err(bad(
                ErrorCode::InvalidArgument,
                "views.name",
                "view name is empty or too long",
            ));
        }
        if name.chars().any(char::is_control) {
            return Err(bad(
                ErrorCode::InvalidArgument,
                "views.name",
                "view name has control characters",
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            plane,
            pose,
            work_area,
        })
    }
}

/// The named-view registry of one model: names to views, ordered by name, bounded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewBook {
    views: BTreeMap<String, NamedView>,
}

impl ViewBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Saves (inserts or replaces) a view; refuses beyond [`MAX_VIEWS`] new names.
    pub fn save(&mut self, view: NamedView) -> Result<(), CommandError> {
        if !self.views.contains_key(&view.name) && self.views.len() >= MAX_VIEWS {
            return Err(bad(
                ErrorCode::LimitExceeded,
                "views.count",
                "the model already has the maximum number of saved views",
            ));
        }
        self.views.insert(view.name.clone(), view);
        Ok(())
    }

    /// Removes a view by name, returning it.
    pub fn remove(&mut self, name: &str) -> Option<NamedView> {
        self.views.remove(name)
    }

    /// Looks a view up by name.
    pub fn get(&self, name: &str) -> Option<&NamedView> {
        self.views.get(name)
    }

    /// Views in name order.
    pub fn iter(&self) -> impl Iterator<Item = &NamedView> {
        self.views.values()
    }

    /// How many views are saved.
    pub fn len(&self) -> usize {
        self.views.len()
    }

    /// Whether nothing is saved.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

/// Resolves a plane spec to the pose that shows it. `centre_mm` is the frame centre (the scene
/// bounds centre in the window, the caller's choice headlessly) and `distance_mm` the orbit
/// distance to carry into the resolved view; both are admitted by [`ViewPose::new`].
pub fn resolve_plane(
    spec: &PlaneSpec,
    grids: &[Grid],
    centre_mm: [f64; 3],
    distance_mm: f64,
) -> Result<ViewPose, CommandError> {
    let unknown = |message: &'static str| bad(ErrorCode::NotFound, "views.plane", message);
    match spec {
        PlaneSpec::GridLine {
            grid,
            line,
            reverse,
        } => {
            let resolved = grid_query::resolve_grid(grids, grid)?;
            // The plane carries the horizontal normal; the target sits at the line's world
            // midpoint, at the frame centre's height.
            let plane = grid_query::grid_line_plane(resolved, line)?;
            let lines = grid_query::segments(resolved)?;
            let found = lines
                .iter()
                .find(|l| l.label == *line)
                .ok_or_else(|| unknown("GRID_AXIS_UNKNOWN"))?;
            let a = grid_query::local_to_world(resolved, found.start);
            let b = grid_query::local_to_world(resolved, found.end);
            let side = if *reverse { -1.0 } else { 1.0 };
            let back = [plane.normal_world[0] * side, plane.normal_world[1] * side];
            ViewPose::new(
                [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, centre_mm[2]],
                back[1].atan2(back[0]),
                0.0,
                distance_mm,
                false,
            )
        }
        PlaneSpec::Level => ViewPose::new(centre_mm, 0.0, PITCH_MAX_RAD, distance_mm, false),
        PlaneSpec::TwoPoints { from_mm, to_mm } => {
            for point in [from_mm, to_mm] {
                if point
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > TARGET_CAP_MM)
                {
                    return Err(bad(
                        ErrorCode::LimitExceeded,
                        "views.plane",
                        "plane point exceeds the coordinate cap",
                    ));
                }
            }
            let dir = [
                from_mm[0] - to_mm[0],
                from_mm[1] - to_mm[1],
                from_mm[2] - to_mm[2],
            ];
            let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            if len < 1e-9 {
                return Err(bad(
                    ErrorCode::InvalidArgument,
                    "views.plane",
                    "the two plane points coincide",
                ));
            }
            let back = [dir[0] / len, dir[1] / len, dir[2] / len];
            // A view from below flattens to horizontal: the orbit pitch never goes negative.
            ViewPose::new(
                *to_mm,
                back[1].atan2(back[0]),
                back[2].clamp(0.0, 1.0).asin().clamp(0.0, PITCH_MAX_RAD),
                distance_mm,
                false,
            )
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use beamcraft_model::grid::{DrawnGridLine, GridFamily, RegularGridFamily};

    fn grid_a() -> Grid {
        let mut n: u64 = 0;
        let ids = |count: usize, n: &mut u64| -> Vec<beamcraft_model::id::ElementId> {
            (0..count)
                .map(|_| {
                    *n += 1;
                    beamcraft_model::id::ElementId::parse(&format!(
                        "019a7bf0-0000-7000-8000-{n:012}"
                    ))
                    .unwrap()
                })
                .collect()
        };
        let u_ids = ids(2, &mut n);
        let u = GridFamily::Regular(RegularGridFamily {
            expression: "0,4000".into(),
            axis_ids: u_ids,
            labels: vec!["A".to_owned(), "B".to_owned()],
            positions_mm: vec![0.0, 4000.0],
            span_mm: [0.0, 8000.0],
        });
        let v_ids = ids(2, &mut n);
        let v = GridFamily::Regular(RegularGridFamily {
            expression: "0,6000".into(),
            axis_ids: v_ids,
            labels: vec!["1".to_owned(), "2".to_owned()],
            positions_mm: vec![0.0, 6000.0],
            span_mm: [0.0, 12000.0],
        });
        Grid {
            id: beamcraft_model::id::ElementId::parse("019a7bf0-0000-7000-8000-000000000099")
                .unwrap(),
            name: "Main".into(),
            u,
            v,
            translation_mm: [500.0, -250.0],
            orientation_zw: [0.0, 1.0],
        }
    }

    #[test]
    fn poses_admit_camera_bounds_only() {
        let ok = ViewPose::new([0.0; 3], 7.0, 0.85, 3000.0, false).unwrap();
        assert_eq!(ok.yaw, 7.0 - std::f64::consts::TAU);
        assert!(ViewPose::new([f64::NAN; 3], 0.0, 0.1, 100.0, false).is_err());
        assert!(ViewPose::new([0.0; 3], 0.0, 1.6, 100.0, false).is_err());
        assert!(ViewPose::new([0.0; 3], 0.0, 0.0, 5.0, false).is_err());
        assert!(ViewPose::new([1e8, 0.0, 0.0], 0.0, 0.1, 100.0, false).is_err());
        ViewPose::new([1e7; 3], 0.0, PITCH_MAX_RAD, 2e7, true).unwrap();
    }

    #[test]
    fn work_areas_must_be_finite_and_ordered() {
        assert!(WorkArea::new([0.0; 3], [100.0; 3]).is_ok());
        assert!(WorkArea::new([100.0; 3], [0.0; 3]).is_err());
        assert!(WorkArea::new([f64::INFINITY; 3], [0.0; 3]).is_err());
        // The cap is the shared view_clipping one, not the orbit camera's.
        assert!(WorkArea::new([0.0; 3], [2e7; 3]).is_ok());
        assert!(WorkArea::new([0.0; 3], [2e9; 3]).is_err());
    }

    #[test]
    fn names_are_trimmed_short_labels() {
        let pose = ViewPose::new([0.0; 3], 0.0, 0.1, 100.0, false).unwrap();
        assert_eq!(NamedView::new("  A  ", None, pose, None).unwrap().name, "A");
        assert!(NamedView::new("   ", None, pose, None).is_err());
        assert!(NamedView::new(&"x".repeat(65), None, pose, None).is_err());
        assert!(NamedView::new("a\nb", None, pose, None).is_err());
    }

    #[test]
    fn the_book_is_bounded_and_ordered() {
        let mut book = ViewBook::new();
        let pose = ViewPose::new([0.0; 3], 0.0, 0.1, 100.0, false).unwrap();
        book.save(
            NamedView::new(
                "Plan",
                Some(PlaneSpec::Level),
                pose,
                Some(WorkArea::new([0.0; 3], [100.0; 3]).unwrap()),
            )
            .unwrap(),
        )
        .unwrap();
        book.save(NamedView::new("3D", None, pose, None).unwrap())
            .unwrap();
        assert_eq!(book.iter().next().unwrap().name, "3D");
        book.save(NamedView::new("Plan", Some(PlaneSpec::Level), pose, None).unwrap())
            .unwrap();
        assert_eq!(book.len(), 2);
        // The first save carried a work area; the replacement dropped it.
        assert!(book.get("Plan").unwrap().work_area.is_none());
        assert!(book.remove("3D").is_some());
        assert!(book.get("3D").is_none());
        let mut full = ViewBook::new();
        for i in 0..128 {
            full.save(NamedView::new(&format!("v{i}"), None, pose, None).unwrap())
                .unwrap();
        }
        assert!(
            full.save(NamedView::new("over", None, pose, None).unwrap())
                .is_err()
        );
    }

    #[test]
    fn a_grid_line_view_faces_the_line_plane_at_its_midpoint() {
        let grids = vec![grid_a()];
        let spec = PlaneSpec::GridLine {
            grid: "Main".into(),
            line: "A".into(),
            reverse: false,
        };
        let pose = resolve_plane(&spec, &grids, [2000.0, 1000.0, 300.0], 2500.0).unwrap();
        // Line A is local x = 0, world x = 500; the midpoint of the span is local (0, 4000).
        assert_eq!(pose.target_mm, [500.0, 3750.0, 300.0]);
        // The normal of the A plane points to -x (line direction is +y, normal [-dy, dx]).
        assert!(
            (pose.yaw - std::f64::consts::PI).abs() < 1e-9,
            "{}",
            pose.yaw
        );
        assert_eq!(pose.pitch, 0.0);
        // Reverse looks from the other side: yaw is 0, not pi.
        let back = resolve_plane(
            &PlaneSpec::GridLine {
                grid: "Main".into(),
                line: "A".into(),
                reverse: true,
            },
            &grids,
            [0.0; 3],
            2500.0,
        )
        .unwrap();
        assert!(back.yaw.abs() < 1e-9, "{}", back.yaw);
        // Unknown names refuse, they never invent a view.
        assert!(
            resolve_plane(
                &PlaneSpec::GridLine {
                    grid: "Other".into(),
                    line: "A".into(),
                    reverse: false,
                },
                &grids,
                [0.0; 3],
                100.0
            )
            .is_err()
        );
        assert!(
            resolve_plane(
                &PlaneSpec::GridLine {
                    grid: "Main".into(),
                    line: "Z".into(),
                    reverse: false,
                },
                &grids,
                [0.0; 3],
                100.0
            )
            .is_err()
        );
    }

    #[test]
    fn level_and_two_point_views_resolve_from_the_frame() {
        let pose = resolve_plane(&PlaneSpec::Level, &[], [1000.0, 2000.0, 300.0], 4000.0).unwrap();
        assert_eq!(pose.target_mm, [1000.0, 2000.0, 300.0]);
        assert_eq!(pose.pitch, PITCH_MAX_RAD);
        assert_eq!(pose.yaw, 0.0);
        let down = resolve_plane(
            &PlaneSpec::TwoPoints {
                from_mm: [0.0, 0.0, 5000.0],
                to_mm: [4000.0, 0.0, 0.0],
            },
            &[],
            [0.0; 3],
            4000.0,
        )
        .unwrap();
        assert_eq!(down.target_mm, [4000.0, 0.0, 0.0]);
        let want = (5000.0f64 / 41_000_000f64.sqrt()).asin();
        assert!((down.pitch - want).abs() < 1e-9);
        // A view from below flattens to horizontal.
        let up = resolve_plane(
            &PlaneSpec::TwoPoints {
                from_mm: [0.0, 0.0, -5000.0],
                to_mm: [0.0, 0.0, 0.0],
            },
            &[],
            [0.0; 3],
            4000.0,
        )
        .unwrap();
        assert_eq!(up.pitch, 0.0);
        assert!(
            resolve_plane(
                &PlaneSpec::TwoPoints {
                    from_mm: [1.0; 3],
                    to_mm: [1.0; 3]
                },
                &[],
                [0.0; 3],
                4000.0
            )
            .is_err()
        );
    }

    #[test]
    fn a_drawn_line_resolves_like_a_regular_one() {
        let mut grid = grid_a();
        grid.u = GridFamily::Drawn(vec![DrawnGridLine {
            axis_id: beamcraft_model::id::ElementId::parse("019a7bf0-0000-7000-8000-000000000777")
                .unwrap(),
            label: "C".into(),
            start_mm: [1000.0, 2000.0],
            end_mm: [1000.0, 9000.0],
        }]);
        let pose = resolve_plane(
            &PlaneSpec::GridLine {
                grid: "Main".into(),
                line: "C".into(),
                reverse: true,
            },
            std::slice::from_ref(&grid),
            [0.0; 3],
            1500.0,
        )
        .unwrap();
        assert_eq!(pose.target_mm, [1500.0, 5250.0, 0.0]);
        assert!(pose.yaw.abs() < 1e-9, "{}", pose.yaw);
    }
}

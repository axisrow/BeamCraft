// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Orbit camera: spherical orbit, pan and zoom around a target, a perspective or an
//! orthographic projection (V13), right-handed Z up, millimetres. Display only; derived from
//! the V01 spike.
use crate::Error;
/// Bounded orbit/pan/zoom state; fields private to preserve finiteness.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    target: [f32; 3],
    yaw: f32,
    pitch: f32,
    distance: f32,
    orthographic: bool,
}
/// Validated camera basis; four-component vectors match WGSL uniform alignment.
#[derive(Debug, Clone, Copy)]
pub struct View {
    /// Screen right and padding.
    pub right: [f32; 4],
    /// Screen up and padding.
    pub up: [f32; 4],
    /// From target to eye and padding.
    pub back: [f32; 4],
    /// Eye position and padding.
    pub eye: [f32; 4],
    /// Perspective: tan(vertical FOV/2), aspect, near and far. Orthographic keeps the same
    /// layout with the tan replaced by the half-height in millimetres, so one shader divides
    /// by it in both.
    pub params: [f32; 4],
    /// 1.0 orthographic, 0.0 perspective, then padding.
    pub mode: [f32; 4],
}
fn dot([x, y, z]: [f32; 3], [a, b, c]: [f32; 3]) -> f32 {
    x * a + y * b + z * c
}
impl Camera {
    /// Frames a bounded scene for a viewport of `aspect` (width / height): `extent` is the
    /// diagonal of its bounds in millimetres. The bounding sphere fits both the vertical and the
    /// horizontal field of view, so a portrait viewport is framed as well as a landscape one.
    pub fn new(target: [f32; 3], extent: f32, aspect: f32) -> Result<Self, Error> {
        if !target.iter().all(|x| x.is_finite() && x.abs() <= 1e7)
            || !extent.is_finite()
            || !(1.0..=1e7).contains(&extent)
            || !aspect.is_finite()
            || !(0.05..=20.0).contains(&aspect)
        {
            return Err(Error::Input("camera framing"));
        }
        let vertical = std::f32::consts::PI / 8.0;
        let horizontal = (vertical.tan() * aspect).atan();
        let half = vertical.min(horizontal);
        Ok(Self {
            target,
            yaw: -0.8,
            pitch: 0.85,
            distance: (extent / 2.0 / half.sin() * 1.05).clamp(10.0, 2e7),
            orthographic: false,
        })
    }
    /// Switches the projection without moving the view: the orthographic window keeps the
    /// perspective image's half-height at the target (`distance * tan(half FOV)`), so the model
    /// does not jump, and zoom, pan and the distance bounds mean the same in both because the
    /// orthographic scale always follows the distance.
    pub fn set_projection(&mut self, orthographic: bool) {
        self.orthographic = orthographic;
    }
    /// True while the orthographic projection is shown.
    pub fn is_orthographic(&self) -> bool {
        self.orthographic
    }
    /// Orbit (radians), pan (screen-relative mm) and logarithmic zoom; reject hostile inputs.
    pub fn navigate(&mut self, orbit: [f32; 2], pan: [f32; 2], zoom: f32) -> Result<(), Error> {
        if !orbit
            .into_iter()
            .chain(pan)
            .chain([zoom])
            .all(|x| x.is_finite() && x.abs() <= 1e6)
        {
            return Err(Error::Input("camera navigation"));
        }
        let [ox, oy] = orbit;
        let [px, py] = pan;
        self.yaw = (self.yaw + ox).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + oy).clamp(0.08, 1.52);
        self.distance = (self.distance * (-zoom.clamp(-10.0, 10.0)).exp()).clamp(10.0, 2e7);
        let view = self.view(1.0)?;
        let [rx, ry, rz, _] = view.right;
        let [ux, uy, uz, _] = view.up;
        let [x, y, z] = self.target;
        self.target = [
            (x + rx * px + ux * py).clamp(-1e7, 1e7),
            (y + ry * px + uy * py).clamp(-1e7, 1e7),
            (z + rz * px + uz * py).clamp(-1e7, 1e7),
        ];
        Ok(())
    }
    /// Frames new bounds for a viewport of `aspect` while keeping the viewing direction.
    pub fn fit(&mut self, target: [f32; 3], extent: f32, aspect: f32) -> Result<(), Error> {
        let framed = Self::new(target, extent, aspect)?;
        self.target = framed.target;
        self.distance = framed.distance;
        Ok(())
    }
    /// Distance used to convert UI pan pixels to display millimetres.
    pub fn distance(&self) -> f32 {
        self.distance
    }
    /// The pan (millimetres) that keeps the point under the cursor fixed while zooming: `ndc` is
    /// the cursor in normalized device coordinates (`edit::ndc_of`, y up, inside `[-1, 1]`) and
    /// `zoom` the same logarithmic input `navigate` takes. On the target plane the view scales
    /// from the distance `navigate` will actually reach (including its clamps), so moving the
    /// target by `(1 - s)` of the cursor's offset from the target keeps the anchor still even at
    /// the zoom bounds; zero zoom gives zero pan.
    pub fn cursor_zoom_pan(
        &self,
        ndc: [f32; 2],
        aspect: f32,
        zoom: f32,
    ) -> Result<[f32; 2], Error> {
        if !ndc.into_iter().all(|x| x.is_finite())
            || ndc[0].abs() > 1.0
            || ndc[1].abs() > 1.0
            || !zoom.is_finite()
            || !aspect.is_finite()
            || !(0.05..=20.0).contains(&aspect)
        {
            return Err(Error::Input("camera cursor zoom"));
        }
        // The vertical field angle, not `view.params[0]`: under the orthographic projection
        // that slot carries a length in millimetres and the distance below would enter twice.
        let tan = (std::f32::consts::PI / 8.0).tan();
        let d = self.distance;
        let reached = (d * (-zoom.clamp(-10.0, 10.0)).exp()).clamp(10.0, 2e7);
        let s = reached / d;
        let nx = ndc[0].clamp(-1.0, 1.0);
        let ny = ndc[1].clamp(-1.0, 1.0);
        // Clamp each component into what navigate accepts: near the distance bounds the raw
        // compensation (up to d * tan * aspect) would get the whole frame rejected.
        Ok([
            ((1.0 - s) * d * nx * tan * aspect).clamp(-1e6, 1e6),
            ((1.0 - s) * d * ny * tan).clamp(-1e6, 1e6),
        ])
    }
    /// Perspective or orthographic basis with depth range `[0, 1]`, vertical FOV 45 degrees.
    pub fn view(&self, aspect: f32) -> Result<View, Error> {
        if !aspect.is_finite() || !(0.05..=20.0).contains(&aspect) {
            return Err(Error::Input("camera aspect"));
        }
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let back = [cy * cp, sy * cp, sp, 0.0];
        let right = [-sy, cy, 0.0, 0.0];
        let up = [-cy * sp, -sy * sp, cp, 0.0];
        let [x, y, z] = self.target;
        let [bx, by, bz, _] = back;
        let tan = (std::f32::consts::PI / 8.0).tan();
        // The near plane follows the camera distance. A fixed 1 mm near plane leaves a
        // `Depth32Float` buffer too little resolution over a long model (far is at least
        // 1e7 mm), so hidden parts showed through; one percent of the distance keeps
        // precision where the orbit camera can be anyway.
        let near = (self.distance * 0.01).clamp(0.1, 1e5);
        Ok(View {
            right,
            up,
            back,
            eye: [
                x + bx * self.distance,
                y + by * self.distance,
                z + bz * self.distance,
                0.0,
            ],
            // Orthographic replaces the field angle with the half-height of the image at the
            // target plane, so the same `dot / scale` division draws both projections.
            params: [
                if self.orthographic {
                    self.distance * tan
                } else {
                    tan
                },
                aspect,
                near,
                self.distance * 5.0 + 1e7,
            ],
            mode: [u8::from(self.orthographic) as f32, 0.0, 0.0, 0.0],
        })
    }
}
impl View {
    /// CPU equivalent of shader projection, for camera and pixel-test evidence.
    pub fn project(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        let [ex, ey, ez, _] = self.eye;
        let p = [x - ex, y - ey, z - ez];
        let [rx, ry, rz, _] = self.right;
        let [ux, uy, uz, _] = self.up;
        let [bx, by, bz, _] = self.back;
        let [scale, aspect, near, far] = self.params;
        let depth = -dot(p, [bx, by, bz]);
        // Perspective divides by the view depth; orthographic divides by the constant window
        // the params carry (half-height, aspect), so nothing shrinks with distance. The depth
        // matches the shader: clip z over w there, linear over the depth range here.
        let [sx, sy] = if self.mode[0] > 0.5 {
            [scale * aspect, scale]
        } else {
            [depth * scale * aspect, depth * scale]
        };
        let z = if self.mode[0] > 0.5 {
            (depth - near) / (far - near)
        } else {
            far / (far - near) - far * near / ((far - near) * depth)
        };
        [dot(p, [rx, ry, rz]) / sx, dot(p, [ux, uy, uz]) / sy, z]
    }
    pub(crate) fn bytes(self) -> Vec<u8> {
        self.right
            .into_iter()
            .chain(self.up)
            .chain(self.back)
            .chain(self.eye)
            .chain(self.params)
            .chain(self.mode)
            .flat_map(f32::to_le_bytes)
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_fitted_scene_projects_inside_the_view() {
        // A 6 m x 3 m x 0.5 m box centred at the origin, in millimetres, in landscape and portrait.
        let half = [3000.0f32, 1500.0, 250.0];
        let extent = 2.0 * (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
        for aspect in [16.0 / 9.0, 1.0, 0.4] {
            let mut c = Camera::new([0.0; 3], 10.0, aspect).unwrap();
            c.navigate([0.7, -0.2], [0.0; 2], 0.0).unwrap();
            c.fit([0.0; 3], extent, aspect).unwrap();
            let v = c.view(aspect).unwrap();
            for sx in [-1.0f32, 1.0] {
                for sy in [-1.0f32, 1.0] {
                    for sz in [-1.0f32, 1.0] {
                        let [x, y, z] = v.project([sx * half[0], sy * half[1], sz * half[2]]);
                        assert!(
                            x.abs() <= 1.0 && y.abs() <= 1.0 && (0.0..1.0).contains(&z),
                            "aspect {aspect}: {x} {y} {z}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn projection_and_basis() {
        let c = Camera::new([0.0; 3], 100.0, 2.0).unwrap();
        let v = c.view(2.0).unwrap();
        let [x, y, z] = v.project([0.0; 3]);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5 && (0.0..1.0).contains(&z));
        let [rx, ry, rz, _] = v.right;
        let [ux, uy, uz, _] = v.up;
        let [bx, by, bz, _] = v.back;
        assert!(dot([rx, ry, rz], [ux, uy, uz]).abs() < 1e-6);
        assert!(dot([rx, ry, rz], [bx, by, bz]).abs() < 1e-6);
        for a in [[rx, ry, rz], [ux, uy, uz], [bx, by, bz]] {
            assert!((dot(a, a) - 1.0).abs() < 1e-6);
        }
        let [px, _, _] = v.project([rx * 10.0, ry * 10.0, rz * 10.0]);
        assert!(px > 0.0);
    }
    #[test]
    fn input_bounds_and_navigation() {
        assert!(Camera::new([f32::NAN; 3], 1.0, 1.0).is_err());
        assert!(Camera::new([0.0; 3], 100.0, 0.0).is_err());
        let mut c = Camera::new([0.0; 3], 100.0, 1.0).unwrap();
        assert!(c.navigate([f32::INFINITY, 0.0], [0.0; 2], 0.0).is_err());
        assert!(c.view(0.0).is_err());
        for _ in 0..100 {
            c.navigate([100.0, 100.0], [1e6, -1e6], 100.0).unwrap();
        }
        assert!(c.view(1.0).unwrap().bytes().len() == 96);
        assert!(c.distance().is_finite());
    }
    #[test]
    fn zoom_toward_the_cursor_keeps_the_point_under_the_cursor_still() {
        let aspect = 1.6;
        // A fresh camera per case: the invariant must not depend on navigation history.
        for ndc in [[0.5f32, 0.3], [-0.7, -0.4], [0.9, -0.8], [0.0, 0.0]] {
            for zoom in [0.7f32, -0.9, 2.0] {
                let mut c = Camera::new([0.0; 3], 1000.0, aspect).unwrap();
                let v = c.view(aspect).unwrap();
                let [tan, va, _, _] = v.params;
                let d = c.distance();
                // The anchor is where the cursor ray crosses the plane through the target.
                let anchor = [
                    v.eye[0] - v.back[0] * d
                        + v.right[0] * ndc[0] * tan * va * d
                        + v.up[0] * ndc[1] * tan * d,
                    v.eye[1] - v.back[1] * d
                        + v.right[1] * ndc[0] * tan * va * d
                        + v.up[1] * ndc[1] * tan * d,
                    v.eye[2] - v.back[2] * d
                        + v.right[2] * ndc[0] * tan * va * d
                        + v.up[2] * ndc[1] * tan * d,
                ];
                let want = v.project(anchor);
                let pan = c.cursor_zoom_pan(ndc, aspect, zoom).unwrap();
                c.navigate([0.0; 2], pan, zoom).unwrap();
                let got = c.view(aspect).unwrap().project(anchor);
                assert!(
                    (want[0] - got[0]).abs() < 1e-3 && (want[1] - got[1]).abs() < 1e-3,
                    "ndc {ndc:?} zoom {zoom}: {want:?} -> {got:?}"
                );
            }
        }
        // At the zoom bounds navigate clamps the distance; the compensation follows the clamped
        // distance, so the anchor still stays still (a raw exponential here would miss).
        let mut c = Camera::new([0.0; 3], 1000.0, aspect).unwrap();
        let zoom = 6.0f32;
        let v = c.view(aspect).unwrap();
        let [tan, va, _, _] = v.params;
        let d = c.distance();
        let anchor = [
            v.eye[0] - v.back[0] * d + v.right[0] * 0.5 * tan * va * d + v.up[0] * 0.3 * tan * d,
            v.eye[1] - v.back[1] * d + v.right[1] * 0.5 * tan * va * d + v.up[1] * 0.3 * tan * d,
            v.eye[2] - v.back[2] * d + v.right[2] * 0.5 * tan * va * d + v.up[2] * 0.3 * tan * d,
        ];
        let want = v.project(anchor);
        let pan = c.cursor_zoom_pan([0.5, 0.3], aspect, zoom).unwrap();
        c.navigate([0.0; 2], pan, zoom).unwrap();
        assert_eq!(c.distance(), 10.0); // the raw exponential would have gone to 3.4
        let got = c.view(aspect).unwrap().project(anchor);
        assert!(
            (want[0] - got[0]).abs() < 1e-3 && (want[1] - got[1]).abs() < 1e-3,
            "clamped: {want:?} -> {got:?}"
        );
        // The compensation is clamped into what navigate accepts: at the distance bounds it
        // would otherwise exceed the 1e6 input limit and get the whole frame rejected.
        let mut big = Camera::new([0.0; 3], 1e7, aspect).unwrap();
        let pan = big.cursor_zoom_pan([1.0, 1.0], aspect, 0.4).unwrap();
        assert!(pan[0].abs() <= 1e6 && pan[1].abs() <= 1e6, "{pan:?}");
        assert!(big.navigate([0.0; 2], pan, 0.4).is_ok());
        assert!(c.cursor_zoom_pan([f32::NAN, 0.0], aspect, 1.0).is_err());
        assert!(c.cursor_zoom_pan([2.0, 0.0], aspect, 1.0).is_err());
        assert!(c.cursor_zoom_pan([0.0, 0.0], aspect, f32::NAN).is_err());
    }
    #[test]
    fn the_near_plane_follows_the_distance_and_stays_inside_the_depth_range() {
        for extent in [10.0f32, 1000.0, 1e6] {
            let c = Camera::new([0.0; 3], extent, 1.6).unwrap();
            let v = c.view(1.6).unwrap();
            let near = v.params[2];
            let far = v.params[3];
            assert!((near - c.distance() * 0.01).abs() < near * 1e-4);
            assert!(near > 0.0 && near < far);
        }
        // Every zoom extreme keeps a valid depth range.
        let mut c = Camera::new([0.0; 3], 1000.0, 1.6).unwrap();
        c.navigate([0.0, 0.0], [0.0; 2], -10.0).unwrap();
        c.navigate([0.0, 0.0], [0.0; 2], 10.0).unwrap();
        let v = c.view(1.6).unwrap();
        assert!(v.params[2] > 0.0 && v.params[2] < v.params[3]);
    }
    #[test]
    fn switching_to_orthographic_keeps_the_image_and_the_navigation_scale() {
        // A point on the plane through the target, perpendicular to the view, lands on the same
        // pixel when the projection flips: the orthographic window keeps the perspective
        // half-height at the target (off that plane a perspective image scales by d/depth).
        let aspect = 1.6;
        let half = [2000.0f32, 1000.0, 100.0];
        let extent = 2.0 * (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
        let mut c = Camera::new([0.0; 3], extent, aspect).unwrap();
        c.navigate([0.7, -0.2], [0.0; 2], 0.0).unwrap();
        let view = c.view(aspect).unwrap();
        let [rx, ry, rz, _] = view.right;
        let [ux, uy, uz, _] = view.up;
        let world = [
            rx * 1200.0 + ux * 600.0,
            ry * 1200.0 + uy * 600.0,
            rz * 1200.0 + uz * 600.0,
        ];
        let before = c.view(aspect).unwrap().project(world);
        c.set_projection(true);
        assert!(c.is_orthographic());
        let ortho = c.view(aspect).unwrap();
        let after = ortho.project(world);
        assert!(
            (before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3,
            "perspective {before:?} -> orthographic {after:?}"
        );
        // The params now carry the half-height in millimetres, and the whole framed box stays
        // inside the view with a depth inside [0, 1].
        assert!((ortho.params[0] - c.distance() * (std::f32::consts::PI / 8.0).tan()).abs() < 1e-3);
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let [x, y, z] = ortho.project([sx * half[0], sy * half[1], sz * half[2]]);
                    assert!(
                        x.abs() <= 1.0 && y.abs() <= 1.0 && (0.0..1.0).contains(&z),
                        "corner {x} {y} {z}"
                    );
                }
            }
        }
        // Navigation keeps working under the orthographic projection: zooming in (distance and
        // window shrink together) makes the model fill more of the view, by the same
        // exponential factor as in perspective.
        let width = ortho.project([half[0], 0.0, 0.0])[0] - ortho.project([-half[0], 0.0, 0.0])[0];
        c.navigate([0.0, 0.0], [0.0; 2], 1.0).unwrap();
        let zoomed = c.view(aspect).unwrap();
        let wider =
            zoomed.project([half[0], 0.0, 0.0])[0] - zoomed.project([-half[0], 0.0, 0.0])[0];
        assert!(wider > width * 2.0, "{width} -> {wider}");
        c.set_projection(false);
        assert!(!c.is_orthographic());
    }
    #[test]
    fn orthographic_input_and_zoom_bounds_stay_bounded() {
        let aspect = 1.6;
        let mut c = Camera::new([0.0; 3], 1000.0, aspect).unwrap();
        c.set_projection(true);
        for _ in 0..100 {
            c.navigate([100.0, 100.0], [1e6, -1e6], 100.0).unwrap();
        }
        let v = c.view(aspect).unwrap();
        assert!(v.params.into_iter().all(|x| x.is_finite() && x > 0.0));
        assert!(v.mode[0] > 0.5);
        // The cursor anchor compensation uses the field angle in both projections: under the
        // orthographic window (a length) the distance would enter twice and overflow.
        let pan = c.cursor_zoom_pan([0.5, 0.3], aspect, 2.0).unwrap();
        assert!(pan.into_iter().all(|x| x.is_finite()));
        assert!(c.cursor_zoom_pan([2.0, 0.0], aspect, 1.0).is_err());
        assert!(c.cursor_zoom_pan([0.0, 0.0], 0.0, 1.0).is_err());
    }
}

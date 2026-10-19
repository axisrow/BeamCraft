// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Orbit camera: spherical orbit, pan and zoom around a target, perspective projection,
//! right-handed Z up, millimetres. Display only; derived from the V01 spike.
use crate::Error;
/// Bounded orbit/pan/zoom state; fields private to preserve finiteness.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    target: [f32; 3],
    yaw: f32,
    pitch: f32,
    distance: f32,
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
    /// tan(vertical FOV/2), aspect, near and far.
    pub params: [f32; 4],
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
        })
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
    /// Perspective basis with depth range `[0, 1]`, vertical FOV 45 degrees.
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
            params: [
                (std::f32::consts::PI / 8.0).tan(),
                aspect,
                1.0,
                self.distance * 5.0 + 1e7,
            ],
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
        let [tan, aspect, near, far] = self.params;
        let depth = -dot(p, [bx, by, bz]);
        [
            dot(p, [rx, ry, rz]) / (depth * tan * aspect),
            dot(p, [ux, uy, uz]) / (depth * tan),
            far / (far - near) - far * near / ((far - near) * depth),
        ]
    }

    pub(crate) fn bytes(self) -> Vec<u8> {
        self.right
            .into_iter()
            .chain(self.up)
            .chain(self.back)
            .chain(self.eye)
            .chain(self.params)
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
        assert!(c.view(1.0).unwrap().bytes().len() == 80);
        assert!(c.distance().is_finite());
    }
}

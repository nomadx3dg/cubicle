//! Minimal 3D vector type. Method style (not operator overloads) to keep the
//! maths readable in the rasteriser's hot loop.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

// Method-style maths is a deliberate choice (see module docs), so silence the lint
// that would prefer the std::ops traits.
#[allow(clippy::should_implement_trait)]
impl V3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        V3 { x, y, z }
    }
    pub fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub fn scale(self, k: f32) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }
    pub fn neg(self) -> V3 {
        self.scale(-1.0)
    }
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        V3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    /// Unit vector, or the input unchanged if it has zero length.
    pub fn normalize(self) -> V3 {
        let l = self.length();
        if l == 0.0 {
            self
        } else {
            self.scale(1.0 / l)
        }
    }
    pub fn rot_x(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(self.x, self.y * c - self.z * s, self.y * s + self.z * c)
    }
    pub fn rot_y(self, a: f32) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_of_axes() {
        let z = V3::new(1.0, 0.0, 0.0).cross(V3::new(0.0, 1.0, 0.0));
        assert_eq!(z, V3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn normalize_zero_is_safe() {
        assert_eq!(V3::default().normalize(), V3::default());
        let n = V3::new(3.0, 0.0, 4.0).normalize();
        assert!((n.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rotation_preserves_length() {
        let v = V3::new(1.0, 2.0, 3.0);
        let r = v.rot_x(0.7).rot_y(-1.3);
        assert!((r.length() - v.length()).abs() < 1e-5);
    }
}

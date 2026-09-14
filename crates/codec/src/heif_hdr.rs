//! Deterministic SDR rendering policy for absolute PQ and reference-display HLG.
use crate::CodecError;
use moxcms::{ColorProfile, Matrix3d, TransferCharacteristics, Vector3d};

pub(crate) struct HdrColour {
    transfer: TransferCharacteristics,
    to_srgb: Matrix3d,
    source_luma: [f64; 3],
}

impl HdrColour {
    pub(crate) fn from_profile(profile: &ColorProfile) -> Result<Option<Self>, CodecError> {
        let Some(cicp) = profile.cicp else {
            return Ok(None);
        };
        if !matches!(
            cicp.transfer_characteristics,
            TransferCharacteristics::Smpte2084 | TransferCharacteristics::Hlg
        ) {
            return Ok(None);
        }
        // HLG's OOTF uses scene luminance in the source RGB primaries.
        let source_luma = match cicp.color_primaries as u8 {
            1 => [0.2126, 0.7152, 0.0722],
            9 => [0.2627, 0.6780, 0.0593],
            12 => [0.2289745641, 0.6917385218, 0.0792869141],
            _ if cicp.transfer_characteristics == TransferCharacteristics::Hlg => {
                return Err(CodecError::Decode {
                    message: "unsupported HLG reference-display primaries".into(),
                });
            }
            _ => [0.2126, 0.7152, 0.0722],
        };
        // Both profile matrices use the same D50 PCS, so adaptation cancels.
        let to_srgb = ColorProfile::new_srgb()
            .rgb_to_xyz_matrix()
            .inverse()
            .mat_mul(profile.rgb_to_xyz_matrix());
        Ok(Some(Self {
            transfer: cicp.transfer_characteristics,
            to_srgb,
            source_luma,
        }))
    }

    pub(crate) fn pixel(&self, encoded: [f64; 3]) -> [u8; 3] {
        let mut light = encoded.map(|value| match self.transfer {
            TransferCharacteristics::Smpte2084 => pq_nits(value),
            _ => hlg_scene_light(value),
        });
        if self.transfer == TransferCharacteristics::Hlg {
            // BT.2100 reference display: 1,000 nit peak, gamma 1.2.
            // Apply one luminance-derived OOTF scale, never per-channel gamma.
            let y = dot(light, self.source_luma).max(0.0);
            let scale = 1000.0 * y.powf(0.2);
            light = light.map(|v| v * scale);
        }
        let linear = self.to_srgb.mul_vector(Vector3d { v: light }).v;
        tone_map(linear).map(srgb_byte)
    }
}

fn dot(rgb: [f64; 3], weights: [f64; 3]) -> f64 {
    rgb.into_iter().zip(weights).map(|(a, b)| a * b).sum()
}

fn pq_nits(encoded: f64) -> f64 {
    // SMPTE ST 2084 EOTF: result is absolute cd/m², not [0,1].
    let p = encoded.clamp(0.0, 1.0).powf(32.0 / 2523.0);
    10_000.0
        * ((p - 3424.0 / 4096.0).max(0.0) / (2413.0 / 128.0 - 2392.0 / 128.0 * p))
            .powf(16384.0 / 2610.0)
}

fn hlg_scene_light(encoded: f64) -> f64 {
    let v = encoded.clamp(0.0, 1.0);
    if v <= 0.5 {
        v * v / 3.0
    } else {
        (((v - 0.55991073) / 0.17883277).exp() + 0.28466892) / 12.0
    }
}

fn tone_map(rgb_nits: [f64; 3]) -> [f64; 3] {
    let y = dot(rgb_nits, [0.2126, 0.7152, 0.0722]);
    if y <= 0.0 {
        return [0.0; 3];
    }
    // 100 nit SDR target. Preserve scene contrast through 50 nits, then use
    // a C1-continuous, monotonic shoulder: 100 nit -> .75, 1000 nit -> .975.
    // It asymptotically approaches white, leaving highlight separation.
    let target = if y <= 50.0 { y / 100.0 } else { 1.0 - 25.0 / y };
    let scaled = rgb_nits.map(|v| v * target / y);
    // Compress out-of-gamut chroma toward the same luminance. A single scale
    // preserves chroma direction/hue; independent channel clipping would not.
    let mut chroma_scale = 1.0f64;
    for v in scaled {
        let delta = v - target;
        if delta > 0.0 {
            chroma_scale = chroma_scale.min((1.0 - target) / delta);
        }
        if delta < 0.0 {
            chroma_scale = chroma_scale.min(-target / delta);
        }
    }
    scaled.map(|v| (target + (v - target) * chroma_scale).clamp(0.0, 1.0))
}

fn srgb_byte(linear: f64) -> u8 {
    let v = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shoulder_preserves_luminance_and_chroma_direction_for_saturated_highlights() {
        for rgb in [
            [1000.0, 100.0, 50.0],
            [20.0, 1000.0, 200.0],
            [-50.0, 250.0, 1000.0],
        ] {
            let y = dot(rgb, [0.2126, 0.7152, 0.0722]);
            let mapped = tone_map(rgb);
            assert!((dot(mapped, [0.2126, 0.7152, 0.0722]) - (1.0 - 25.0 / y)).abs() < 1e-12);
            // A uniform chroma scale preserves the ratio of channel differences.
            assert!(
                ((mapped[0] - mapped[1]) * (rgb[1] - rgb[2])
                    - (mapped[1] - mapped[2]) * (rgb[0] - rgb[1]))
                    .abs()
                    < 1e-10
            );
            assert!(mapped.into_iter().all(|v| (0.0..=1.0).contains(&v)));
        }
    }

    #[test]
    fn reference_neutrals_are_monotonic_and_keep_shadow_precision() {
        for profile in [
            ColorProfile::new_bt2020_pq(),
            ColorProfile::new_bt2020_hlg(),
        ] {
            let hdr = HdrColour::from_profile(&profile).unwrap().unwrap();
            let mut last = 0;
            for code in 0..=1023 {
                let pixel = hdr.pixel([f64::from(code) / 1023.0; 3]);
                assert!(pixel[0] >= last);
                assert_eq!(pixel, [pixel[0]; 3]);
                last = pixel[0];
            }
        }
    }
}

//! The title's output size on Series X (`VICTORY_OUTPUT`).
//!
//! The retail Series X image (1.0.2.1) makes a 3840x2160 output and draws
//! its scene at two thirds of it, 2560x1440, which its own temporal pass
//! then scales up. Run 19 had the card busy about 80 ms of a 140 ms frame;
//! upstream's run 303 found that the display mode we report is not what
//! sizes that output: the title takes the size from its own table of SDR
//! modes. Here the image is patched before it is mapped so that the title
//! makes a smaller output and a smaller scene, with no 4K pass at all:
//!
//! - `1080p` (the default with the faster defaults; the owner's screen is
//!   1080p): a 1920x1080 output with the scene at 1920x1080, drawn at the
//!   screen's own size with no scaling up (under half the pixels of the
//!   stock scene's, a quarter of the stock output's);
//! - `1080p-720p`: a 1920x1080 output, the scene at 1280x720 (two thirds,
//!   as stock) scaled up by the title's temporal pass;
//! - `1440p-1080p`: a 2560x1440 output, the scene at 1920x1080 (the size
//!   other forks have played at);
//! - `1440p`: a 2560x1440 output, the scene at 2560x1440;
//! - `4k` or `0`: the stock image.
//!
//! Four places are changed, each checked against the stock build first
//! (only the build's hash and the places' own stock bytes, as hashes or as
//! the few instruction bytes, are known here; no bytes of the title are in
//! this repository): the SDR mode's width and height, the size of the
//! scanout buffers reserved for it, the temporal pass's scene scale, and a
//! settings screen's viewport load, which reads the legacy 3840x2160 and
//! is pointed at the SDR mode instead. Another build is left as it is.
//!
//! Where the four places are, and what they hold, was found by DexNom
//! (DexNom/VIctory, `native_1440p.rs`, GPL-3.0-or-later) and carried in
//! VIctory-NVIDIA; this is Defeat's own take on it, with the output size
//! as a setting.

use anyhow::{Context, Result, ensure};
use victory_formats::texture::{Dimension, Swizzle, Texture, TextureDesc};
use victory_loader::PreparedImage;

/// The retail Series X build these places are known for.
pub(crate) const RETAIL_BUILD: &str = "b99f876b74d4a77f03af5f878223b27bd647b5c35a79d5decd9f716504239e93";

/// The SDR display mode (its first 16 bytes; width and height first).
const SDR_MODE: u32 = 0x0615_c100;
const STOCK_SDR_MODE: &str = "abfb5438d8153b6a0b8d08f33f4f0066e5383925b55aacc5f669131f3f51d4cf";
/// `mov ecx, <bytes>`: the scanout buffers' reservation.
const SCANOUT_RESERVATION: u32 = 0x0247_ad4a;
/// The temporal pass's scene scale, a float (2/3 stock).
const TEMPORAL_SCALE: u32 = 0x028f_1e7e;
/// A settings screen's `vmovsd xmm1, [rip + d]` of the legacy UI size.
const UI_VIEWPORT_LOAD: u32 = 0x0580_48d4;

/// What `VICTORY_OUTPUT` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Output {
    pub width: u32,
    pub height: u32,
    /// The scene's size.
    pub scene: (u32, u32),
}

/// The output `VICTORY_OUTPUT` asks for; `None` for the stock one.
pub(crate) fn asked() -> Result<Option<Output>> {
    let value = std::env::var("VICTORY_OUTPUT").ok();
    let value = match value.as_deref().map(str::trim) {
        Some(v) => v.to_ascii_lowercase(),
        // The faster defaults (`VICTORY_FAST`, on unless `=0`).
        None if std::env::var_os("VICTORY_FAST").is_none_or(|v| v != "0") => "1080p".to_owned(),
        None => return Ok(None),
    };
    Ok(match value.as_str() {
        "" | "0" | "4k" | "stock" => None,
        "1080p" | "1" => Some(Output { width: 1920, height: 1080, scene: (1920, 1080) }),
        "1080p-720p" => Some(Output { width: 1920, height: 1080, scene: (1280, 720) }),
        "1440p-1080p" => Some(Output { width: 2560, height: 1440, scene: (1920, 1080) }),
        "1440p" => Some(Output { width: 2560, height: 1440, scene: (2560, 1440) }),
        other => anyhow::bail!("VICTORY_OUTPUT={other}: one of 1080p, 1080p-720p, 1440p-1080p, 1440p, 4k"),
    })
}

/// Patches `image` for `output`. Fails, changing nothing, if the image is
/// not the retail build or a place does not hold its stock bytes.
pub(crate) fn prepare(image: &mut PreparedImage, output: Output) -> Result<()> {
    ensure!(
        image.original_hash.to_hex().as_str() == RETAIL_BUILD,
        "the output size is only known for the retail build {RETAIL_BUILD}"
    );
    // Every place checked before any is written.
    ensure!(same(&field::<16>(image, SDR_MODE)?[..], STOCK_SDR_MODE), "unexpected SDR display mode");
    ensure!(*field::<5>(image, SCANOUT_RESERVATION)? == [0xb9, 0, 0, 0, 4], "unexpected scanout reservation");
    ensure!(*field::<4>(image, TEMPORAL_SCALE)? == (2.0f32 / 3.0).to_le_bytes(), "unexpected temporal scene scale");
    ensure!(
        *field::<8>(image, UI_VIEWPORT_LOAD)? == [0xc5, 0xfb, 0x10, 0x0d, 0x10, 0xf0, 0x13, 0x05],
        "unexpected settings viewport load"
    );
    let reservation = scanout_bytes(output.width, output.height)?;

    let mode = field::<16>(image, SDR_MODE)?;
    mode[..4].copy_from_slice(&output.width.to_le_bytes());
    mode[4..8].copy_from_slice(&output.height.to_le_bytes());
    field::<5>(image, SCANOUT_RESERVATION)?[1..].copy_from_slice(&reservation.to_le_bytes());
    *field::<4>(image, TEMPORAL_SCALE)? = (output.scene.0 as f32 / output.width as f32).to_le_bytes();
    let displacement = SDR_MODE as i32 - (UI_VIEWPORT_LOAD as i32 + 8);
    field::<8>(image, UI_VIEWPORT_LOAD)?[4..].copy_from_slice(&displacement.to_le_bytes());
    Ok(())
}

/// The title's renderer choice: `mov eax, 6; cmovne eax, ecx` (6, its own
/// temporal pass on Series X; 7, the FidelityFX path, on Series S).
const RENDERER_SELECTION: u32 = 0x007e_9421;
const STOCK_RENDERER_SELECTION: &str = "f0633abf37df17124108b1e37be64645bd4b2412b0524c2f89334f22bd948464";
/// The FidelityFX path's sizing: the console check's result is noted as
/// the FidelityFX quality mode and picks the scaled branch, whose scale
/// multiplies the output size.
const UPSCALER_SIZING: u32 = 0x028e_f69a;
const STOCK_UPSCALER_SIZING: &str = "2ae2ac102d12ca003cb218413ef018e4dba68fc2003654a30c1743e13a820ffd";
/// The native branch's load of 1.0, and the scaled branch's of the scale.
const NATIVE_LOAD: u32 = 0x028e_f6a8;
const STOCK_NATIVE_LOAD: &str = "2579c24601d173b086a8136edf54ffd82720e6bf186f9139274bb09a45ddadf5";
const SCALED_LOAD: u32 = 0x028e_f6b2;
const STOCK_SCALED_LOAD: &str = "e218882c95d2e16163a409d55b3451519414eb87c4d6220785dc09852d4e14f4";
/// The 2/3 the scaled branch loads, shared with other code (not written).
const UPSCALER_SCALE: u32 = 0x0615_d95c;

/// Makes the title render through its FidelityFX path (renderer 7, as on
/// Series S) with the scene at `scale` of the output, noting `ffx_mode` as
/// its FidelityFX quality mode, so that DLSS can take its upscale
/// dispatches (`victory_hle::modules::fidelityfx`). The temporal pass's
/// scale is set to the same. Every place is checked against the stock
/// bytes first (the temporal scale may be [`prepare`]'s); nothing is
/// written if one differs. The places and the patch are DexNom's finding
/// (DexNom/VIctory, GPL-3.0-or-later), carried in VIctory-NVIDIA.
pub(crate) fn prepare_upscaler(image: &mut PreparedImage, scale: f32, ffx_mode: u8) -> Result<()> {
    ensure!(
        image.original_hash.to_hex().as_str() == RETAIL_BUILD,
        "the FidelityFX path is only known for the retail build {RETAIL_BUILD}"
    );
    ensure!(same(&field::<8>(image, RENDERER_SELECTION)?[..], STOCK_RENDERER_SELECTION), "unexpected renderer selection");
    ensure!(same(&field::<14>(image, UPSCALER_SIZING)?[..], STOCK_UPSCALER_SIZING), "unexpected upscaler sizing");
    ensure!(same(&field::<10>(image, NATIVE_LOAD)?[..], STOCK_NATIVE_LOAD), "unexpected native scale load");
    ensure!(same(&field::<8>(image, SCALED_LOAD)?[..], STOCK_SCALED_LOAD), "unexpected scaled load");
    ensure!(*field::<4>(image, UPSCALER_SCALE)? == (2.0f32 / 3.0).to_le_bytes(), "unexpected upscaler scale");
    let temporal = f32::from_le_bytes(*field::<4>(image, TEMPORAL_SCALE)?);
    ensure!(temporal > 0.0 && temporal <= 1.0, "unexpected temporal scene scale {temporal}");
    ensure!(scale > 0.0 && scale <= 1.0, "a scene scale of {scale}");
    // `mov eax, 7` every time it is asked (the console check has run).
    *field::<8>(image, RENDERER_SELECTION)? = [0xb8, 7, 0, 0, 0, 0x90, 0x90, 0x90];
    // `mov eax, mode; mov [r14+0x90], eax; jmp` to the scaled branch.
    *field::<14>(image, UPSCALER_SIZING)? = [0xb8, ffx_mode, 0, 0, 0, 0x41, 0x89, 0x86, 0x90, 0, 0, 0, 0xeb, 0x0a];
    if scale != 2.0 / 3.0 {
        // The jump leaves the native branch unreached: its load's operand
        // becomes a literal of ours, which the scaled branch loads instead.
        let literal = NATIVE_LOAD + 4;
        *field::<4>(image, literal)? = scale.to_le_bytes();
        let displacement = literal as i32 - (SCALED_LOAD as i32 + 8);
        *field::<4>(image, SCALED_LOAD + 4)? = displacement.to_le_bytes();
    }
    *field::<4>(image, TEMPORAL_SCALE)? = scale.to_le_bytes();
    Ok(())
}

/// The bytes the title reserves for two scanout buffers of this size:
/// tiled RGBA8 (`SW_64KB_D_X`), each 64 KiB aligned, the two rounded up to
/// 2 MiB, as its own allocation check does.
fn scanout_bytes(width: u32, height: u32) -> Result<u32> {
    let texture = Texture::new(&TextureDesc {
        dimension: Dimension::Tex2d,
        width,
        height,
        depth_or_array: 1,
        mips: 1,
        format: 28,
        samples: 1,
        swizzle: Swizzle::SW_64KB_D_X,
    })
    .map_err(|e| anyhow::anyhow!("scanout layout: {e:?}"))?;
    Ok(u32::try_from((texture.size.next_multiple_of(64 << 10) * 2).next_multiple_of(2 << 20))?)
}

/// Whether `bytes` are those whose BLAKE3 hash (hex) is `hash`.
fn same(bytes: &[u8], hash: &str) -> bool {
    blake3::hash(bytes).to_hex().as_str() == hash
}

fn field<const N: usize>(image: &mut PreparedImage, rva: u32) -> Result<&mut [u8; N]> {
    let offset = image.info.rva_to_file_offset(rva).context("output field not in the image")? as usize;
    image
        .bytes
        .get_mut(offset..offset + N)
        .context("output field past the image's end")?
        .try_into()
        .context("output field's length")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanout_reservations_are_whole_two_mib_blocks() {
        for (w, h) in [(2560, 1440), (1920, 1080), (3840, 2160)] {
            let bytes = scanout_bytes(w, h).unwrap();
            assert_eq!(bytes % (2 << 20), 0);
            // Room for two buffers of four bytes a pixel.
            assert!(u64::from(bytes) >= 2 * 4 * u64::from(w) * u64::from(h));
        }
        // A smaller output reserves less than the 4K one.
        assert!(scanout_bytes(2560, 1440).unwrap() < scanout_bytes(3840, 2160).unwrap());
    }

    /// With the retail executable named by `VICTORY_OUTPUT_TEST_IMAGE`
    /// (not part of the repository): each output's four places, and a
    /// second patch refused.
    #[test]
    fn patches_the_retail_image() {
        let Some(path) = std::env::var_os("VICTORY_OUTPUT_TEST_IMAGE") else {
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        for output in [
            Output { width: 2560, height: 1440, scene: (1920, 1080) },
            Output { width: 2560, height: 1440, scene: (2560, 1440) },
            Output { width: 1920, height: 1080, scene: (1280, 720) },
            Output { width: 1920, height: 1080, scene: (1920, 1080) },
        ] {
            let mut image = victory_loader::prepare(&bytes).unwrap();
            prepare(&mut image, output).unwrap();
            let mode = *field::<16>(&mut image, SDR_MODE).unwrap();
            assert_eq!(mode[..4], output.width.to_le_bytes());
            assert_eq!(mode[4..8], output.height.to_le_bytes());
            let scale = f32::from_le_bytes(*field::<4>(&mut image, TEMPORAL_SCALE).unwrap());
            assert_eq!((output.width as f32 * scale).round() as u32, output.scene.0);
            let load = *field::<8>(&mut image, UI_VIEWPORT_LOAD).unwrap();
            let displacement = i32::from_le_bytes(load[4..].try_into().unwrap());
            assert_eq!(UI_VIEWPORT_LOAD.wrapping_add(8).wrapping_add_signed(displacement), SDR_MODE);
            assert!(prepare(&mut image, output).is_err());
            // Then the FidelityFX path at each DLSS scale.
            for (scale, mode) in [(1.0f32, 0u8), (2.0 / 3.0, 1), (0.58, 2), (0.5, 3)] {
                let mut image = image.clone();
                prepare_upscaler(&mut image, scale, mode).unwrap();
                let sizing = *field::<14>(&mut image, UPSCALER_SIZING).unwrap();
                assert_eq!((sizing[1], &sizing[12..]), (mode, &[0xeb, 0x0a][..]));
                // The jump lands on the scaled load, which loads the scale.
                assert_eq!(UPSCALER_SIZING + 14 + 0x0a, SCALED_LOAD);
                let displacement = i32::from_le_bytes(*field::<4>(&mut image, SCALED_LOAD + 4).unwrap());
                let loaded = (SCALED_LOAD + 8).wrapping_add_signed(displacement);
                assert_eq!(f32::from_le_bytes(*field::<4>(&mut image, loaded).unwrap()), scale);
                assert!(prepare_upscaler(&mut image, scale, mode).is_err());
            }
        }
    }
}

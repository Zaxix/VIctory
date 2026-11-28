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
        }
    }
}

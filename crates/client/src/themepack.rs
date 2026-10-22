//! Theme packs: a whole theme folder (theme.toml + PNG textures) in one
//! `.lgtheme` file you can send to someone. Files come from strangers, so
//! everything is validated *before* anything is written: names, sizes, the
//! theme.toml and every PNG; installing is all-or-nothing.
//!
//! Format: `LGTH`, u16 version (little endian), then a zlib stream of
//! `count: u8` entries `{ name_len: u8, name, data_len: u32 LE, data }`.

use crate::textures::{self, MAX_FILE_BYTES};
use crate::theme::{self, Theme};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"LGTH";
const VERSION: u16 = 1;
/// Most bytes a pack may inflate to.
pub const MAX_INFLATED: usize = 24 * 1024 * 1024;
/// Largest pack file accepted.
pub const MAX_PACK_FILE: u64 = 32 * 1024 * 1024;
const MAX_THEME_TOML: usize = 64 * 1024;

type Files = Vec<(String, Vec<u8>)>;

/// Pack the theme folder `dir`: theme.toml and every top-level `.png`.
pub fn pack(dir: &Path) -> Result<Vec<u8>, String> {
    let mut files: Files = Vec::new();
    let toml = std::fs::read(dir.join("theme.toml")).map_err(|e| format!("theme.toml: {e}"))?;
    files.push(("theme.toml".into(), toml));
    let mut pngs: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".png"))
        .collect();
    pngs.sort();
    for n in pngs {
        let data = std::fs::read(dir.join(&n)).map_err(|e| format!("{n}: {e}"))?;
        files.push((n, data));
    }
    let bytes = encode(&files)?;
    // Whatever we write must be readable by us.
    parse(&bytes)?;
    Ok(bytes)
}

fn encode(files: &Files) -> Result<Vec<u8>, String> {
    if files.len() > 255 {
        return Err("too many files".into());
    }
    let mut raw = vec![files.len() as u8];
    for (name, data) in files {
        if name.len() > 255 {
            return Err(format!("{name}: name too long"));
        }
        raw.push(name.len() as u8);
        raw.extend_from_slice(name.as_bytes());
        raw.extend_from_slice(&(data.len() as u32).to_le_bytes());
        raw.extend_from_slice(data);
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(&raw).map_err(|e| e.to_string())?;
    let body = enc.finish().map_err(|e| e.to_string())?;
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode and fully validate a pack. No side effects.
pub fn parse(bytes: &[u8]) -> Result<Files, String> {
    if bytes.len() < 7 || &bytes[..4] != MAGIC {
        return Err("not a theme pack (.lgtheme)".into());
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(format!("theme pack version {version}: update the game"));
    }
    let mut raw = Vec::new();
    ZlibDecoder::new(&bytes[6..])
        .take(MAX_INFLATED as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| "the theme pack is damaged".to_string())?;
    if raw.len() > MAX_INFLATED {
        return Err("the theme pack expands to too much data".into());
    }
    let bad = || "the theme pack is damaged".to_string();
    let mut pos = 0usize;
    let take = |pos: &mut usize, n: usize| -> Result<&[u8], String> {
        let end = pos
            .checked_add(n)
            .filter(|e| *e <= raw.len())
            .ok_or_else(bad)?;
        let s = &raw[*pos..end];
        *pos = end;
        Ok(s)
    };
    let count = take(&mut pos, 1)?[0] as usize;
    let mut files: Files = Vec::new();
    for _ in 0..count {
        let name_len = take(&mut pos, 1)?[0] as usize;
        let name = String::from_utf8(take(&mut pos, name_len)?.to_vec()).map_err(|_| bad())?;
        let len = u32::from_le_bytes(take(&mut pos, 4)?.try_into().unwrap()) as usize;
        let data = take(&mut pos, len)?.to_vec();
        files.push((name, data));
    }
    if pos != raw.len() {
        return Err(bad());
    }
    validate(&files)?;
    Ok(files)
}

/// Names, counts, sizes, theme.toml and every PNG.
fn validate(files: &Files) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    let (mut tomls, mut pngs) = (0, 0);
    for (name, data) in files {
        let lower = name.to_ascii_lowercase();
        if !textures::safe_relative(name) || name.contains('/') {
            return Err(format!("{name:?}: not a plain file name"));
        }
        if lower.contains("theme.toml") && name != "theme.toml" {
            return Err(format!("{name:?}: not an allowed file name"));
        }
        if !seen.insert(lower.clone()) {
            return Err(format!("{name}: appears twice"));
        }
        if name == "theme.toml" {
            tomls += 1;
            if data.len() > MAX_THEME_TOML {
                return Err("theme.toml is too large".into());
            }
        } else if lower.ends_with(".png") && lower.len() > 4 {
            pngs += 1;
            if data.len() as u64 > MAX_FILE_BYTES {
                return Err(format!("{name}: larger than {} KB", MAX_FILE_BYTES / 1024));
            }
        } else {
            return Err(format!(
                "{name}: only theme.toml and .png files are allowed"
            ));
        }
    }
    if tomls != 1 {
        return Err("a theme pack needs exactly one theme.toml".into());
    }
    if pngs > theme::MAX_THEME_TEXTURES {
        return Err(format!(
            "too many textures (at most {})",
            theme::MAX_THEME_TEXTURES
        ));
    }
    let toml = files.iter().find(|(n, _)| n == "theme.toml").unwrap();
    let text = std::str::from_utf8(&toml.1).map_err(|_| "theme.toml isn't text".to_string())?;
    Theme::from_toml(text).map_err(|e| format!("theme.toml: {e}"))?;
    for (name, data) in files
        .iter()
        .filter(|(n, _)| n.to_ascii_lowercase().ends_with(".png"))
    {
        textures::decode_png(data).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

/// Validate `bytes` and install them as `themes_dir/<name>`. Never overwrites an
/// existing theme unless `replace`. Returns the theme folder.
pub fn unpack_to(
    bytes: &[u8],
    themes_dir: &Path,
    name: &str,
    replace: bool,
) -> Result<PathBuf, String> {
    if !theme::is_valid_theme_name(name) || theme::BUILTIN.contains(&name) {
        return Err(format!("{name:?} isn't a usable theme name"));
    }
    let files = parse(bytes)?;
    let target = themes_dir.join(name);
    if target.exists() && !replace {
        return Err(format!(
            "a theme called {name} already exists (import it under another name)"
        ));
    }
    std::fs::create_dir_all(themes_dir).map_err(|e| format!("{}: {e}", themes_dir.display()))?;
    let tmp = themes_dir.join(format!(".import-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&tmp)?;
        for (n, data) in &files {
            std::fs::write(tmp.join(n), data)?;
        }
        if target.exists() {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(&tmp, &target)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("{}: {e}", target.display()));
    }
    Ok(target)
}

/// A theme name from a pack file name (`my theme.lgtheme` → `my-theme`).
pub fn name_from_file(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-').to_string();
    (!name.is_empty() && theme::is_valid_theme_name(&name)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgduel-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn example_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes/example")
    }

    fn pack_of(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        encode(
            &files
                .iter()
                .map(|(n, d)| (n.to_string(), d.clone()))
                .collect(),
        )
        .unwrap()
    }

    fn toml_bytes() -> Vec<u8> {
        b"sky = [0.1, 0.1, 0.1]\n".to_vec()
    }

    #[test]
    fn example_theme_round_trips_and_loads() {
        let bytes = pack(&example_dir()).unwrap();
        let dst = temp("rt");
        let dir = unpack_to(&bytes, &dst, "copy", false).unwrap();
        for (name, data) in parse(&bytes).unwrap() {
            assert_eq!(std::fs::read(dir.join(&name)).unwrap(), data, "{name}");
            assert_eq!(
                std::fs::read(example_dir().join(&name)).unwrap(),
                data,
                "{name}"
            );
        }
        let s = theme::ThemeStyle::load("copy", std::slice::from_ref(&dst));
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.name(), "copy");
        // No temp folders left behind.
        let left: Vec<_> = std::fs::read_dir(&dst)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn hostile_names_are_rejected() {
        let png = textures::encode_png_rgb(1, 1, &[1, 2, 3]);
        for bad in [
            "../x.png",
            "a/b.png",
            "/abs.png",
            "C:\\x.png",
            "theme.toml.png",
            "x.exe",
            "",
            ".png",
            "THEME.TOML",
        ] {
            let r = parse(&pack_of(&[
                ("theme.toml", toml_bytes()),
                (bad, png.clone()),
            ]));
            assert!(r.is_err(), "{bad:?} accepted");
        }
        // Two theme.toml (also differing by case), none at all.
        assert!(
            parse(&pack_of(&[
                ("theme.toml", toml_bytes()),
                ("theme.toml", toml_bytes())
            ]))
            .is_err()
        );
        assert!(parse(&pack_of(&[("a.png", png.clone())])).is_err());
        assert!(
            parse(&pack_of(&[
                ("theme.toml", toml_bytes()),
                ("a.png", png.clone()),
                ("A.PNG", png)
            ]))
            .unwrap_err()
            .contains("twice")
        );
    }

    #[test]
    fn corrupt_content_installs_nothing() {
        let dst = temp("corrupt");
        let good_png = textures::encode_png_rgb(2, 2, &[9; 12]);
        // A bad PNG, a bad theme.toml, an unknown theme key.
        for (toml, png) in [
            (toml_bytes(), b"not a png".to_vec()),
            (b"sky = [".to_vec(), good_png.clone()),
            (b"bogus = 1\n".to_vec(), good_png.clone()),
        ] {
            let bytes = pack_of(&[("theme.toml", toml), ("a.png", png)]);
            assert!(unpack_to(&bytes, &dst, "bad", false).is_err());
            assert!(!dst.join("bad").exists());
        }
        let left: Vec<_> = std::fs::read_dir(&dst).unwrap().flatten().collect();
        assert!(left.is_empty(), "left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn bombs_and_truncation_are_rejected() {
        // Small file, huge inflated size.
        let bomb = pack_of(&[
            ("theme.toml", toml_bytes()),
            ("a.png", vec![0u8; 30 * 1024 * 1024]),
        ]);
        assert!(bomb.len() < 100_000, "{}", bomb.len());
        assert!(parse(&bomb).unwrap_err().contains("too much"));
        // Truncated, wrong magic, wrong version, garbage.
        let ok = pack(&example_dir()).unwrap();
        assert!(parse(&ok[..ok.len() / 2]).is_err());
        assert!(
            parse(b"XXXXxxxxxxxx")
                .unwrap_err()
                .contains("not a theme pack")
        );
        let mut v2 = ok.clone();
        v2[4] = 9;
        assert!(parse(&v2).unwrap_err().contains("version"));
        assert!(parse(&[]).is_err());
        // Oversized PNG entry.
        let big = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        assert!(
            parse(&pack_of(&[("theme.toml", toml_bytes()), ("a.png", big)]))
                .unwrap_err()
                .contains("larger")
        );
    }

    #[test]
    fn collisions_need_replace_and_names_are_checked() {
        let dst = temp("collide");
        let bytes = pack_of(&[("theme.toml", toml_bytes())]);
        unpack_to(&bytes, &dst, "mine", false).unwrap();
        assert!(
            unpack_to(&bytes, &dst, "mine", false)
                .unwrap_err()
                .contains("already exists")
        );
        let newer = pack_of(&[("theme.toml", b"sky = [0.9, 0.9, 0.9]\n".to_vec())]);
        unpack_to(&newer, &dst, "mine", true).unwrap();
        assert!(
            std::fs::read_to_string(dst.join("mine/theme.toml"))
                .unwrap()
                .contains("0.9")
        );
        for bad in ["../evil", "a/b", "classic", "grid", ""] {
            assert!(unpack_to(&bytes, &dst, bad, false).is_err(), "{bad:?}");
        }
        assert_eq!(
            name_from_file(Path::new("/x/My Theme!.lgtheme")).as_deref(),
            Some("My-Theme")
        );
        assert_eq!(name_from_file(Path::new("...lgtheme")), None);
        let _ = std::fs::remove_dir_all(&dst);
    }
}

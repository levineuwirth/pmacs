//! Window geometry persistence (E2.1).
//!
//! The window's logical size and outer position, written under the
//! user's config directory and restored at the next launch. **Logical
//! pixels, never physical**: a geometry saved on a 2× display and
//! restored on a 1× one must open the same apparent window, and winit
//! converts a `LogicalSize` at creation with whatever scale the window
//! lands on.
//!
//! The config directory is resolved by the same rule the core uses for
//! `init.lua` (`src/config.rs::resolve_config_dir`): `$XDG_CONFIG_HOME/
//! pmacs` when set and non-empty, else `$HOME/.config/pmacs`. It is
//! duplicated here rather than linked, because `pmacs-gpu` depends on
//! `pmacs-protocol` and never on `pmacs`; the rule is two lines and
//! pinned by a test on each side.
//!
//! The file is one line, `width height` or `width height x y`, so a
//! person can read or delete it. Anything unparseable is treated as
//! absent, and a stored value outside a sane range is clamped or
//! dropped rather than trusted: a window restored to 0×0 or to a
//! position on a monitor that is gone would be worse than the default.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

/// Default logical extent when nothing is stored: a working editor
/// window rather than the 800×200 strip the frontend once opened at.
pub const DEFAULT_WIDTH: u32 = 1200;
/// See [`DEFAULT_WIDTH`].
pub const DEFAULT_HEIGHT: u32 = 800;

/// File name under the config directory.
pub const FILE_NAME: &str = "gpu-window";

/// The smallest extent a restored window may have on either axis.
const MIN_EXTENT: u32 = 200;
/// The largest extent a restored window may have on either axis.
const MAX_EXTENT: u32 = 16_384;
/// A stored position beyond this magnitude on either axis is dropped.
const MAX_POSITION: i32 = 32_768;

/// The config subdirectory, as in the core.
const CONFIG_SUBDIR: &str = "pmacs";

/// A window's logical geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Logical inner width.
    pub width: u32,
    /// Logical inner height.
    pub height: u32,
    /// Logical outer position, when the platform reports one (Wayland
    /// does not).
    pub position: Option<(i32, i32)>,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            position: None,
        }
    }
}

impl WindowGeometry {
    /// Parse the one-line file format. `None` for anything that is not
    /// `w h` or `w h x y` with integer fields; the result is sanitized.
    pub fn parse(text: &str) -> Option<Self> {
        let mut fields = text.split_whitespace();
        let width: u32 = fields.next()?.parse().ok()?;
        let height: u32 = fields.next()?.parse().ok()?;
        let position = match (fields.next(), fields.next()) {
            (Some(x), Some(y)) => Some((x.parse().ok()?, y.parse().ok()?)),
            (None, None) => None,
            _ => return None,
        };
        if fields.next().is_some() {
            return None;
        }
        Some(
            Self {
                width,
                height,
                position,
            }
            .sanitized(),
        )
    }

    /// The one-line file format, newline-terminated.
    pub fn serialize(&self) -> String {
        match self.position {
            Some((x, y)) => format!("{} {} {x} {y}\n", self.width, self.height),
            None => format!("{} {}\n", self.width, self.height),
        }
    }

    /// Clamp the extent into `[MIN_EXTENT, MAX_EXTENT]` and drop a
    /// position beyond `MAX_POSITION` on either axis.
    pub fn sanitized(self) -> Self {
        Self {
            width: self.width.clamp(MIN_EXTENT, MAX_EXTENT),
            height: self.height.clamp(MIN_EXTENT, MAX_EXTENT),
            position: self
                .position
                .filter(|(x, y)| x.abs() <= MAX_POSITION && y.abs() <= MAX_POSITION),
        }
    }

    /// Read and parse `path`; `None` when absent or unparseable.
    pub fn load(path: &Path) -> Option<Self> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::parse(&text))
    }

    /// Write atomically: the parent is created, the text goes to a
    /// sibling temporary file, and a rename installs it, so a crash
    /// mid-write leaves the previous geometry rather than a torn one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(&tmp, path)
    }
}

/// The core's config-directory rule, factored out so both arms are
/// testable without touching the process environment.
pub fn resolve_config_dir(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(xdg) = xdg
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join(CONFIG_SUBDIR));
    }
    let home = home?;
    Some(PathBuf::from(home).join(".config").join(CONFIG_SUBDIR))
}

/// Where this process's geometry file lives, or `None` when neither
/// `XDG_CONFIG_HOME` nor `HOME` is set, in which case nothing is
/// persisted and the default geometry is used.
pub fn user_geometry_path() -> Option<PathBuf> {
    resolve_config_dir(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .map(|dir| dir.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_a_working_window_not_a_strip() {
        let g = WindowGeometry::default();
        assert_eq!((g.width, g.height), (1200, 800));
        assert_eq!(g.position, None);
    }

    #[test]
    fn size_and_position_round_trip_through_the_file_format() {
        let g = WindowGeometry {
            width: 1024,
            height: 768,
            position: Some((-12, 40)),
        };
        assert_eq!(g.serialize(), "1024 768 -12 40\n");
        assert_eq!(WindowGeometry::parse(&g.serialize()), Some(g));
        let no_pos = WindowGeometry {
            position: None,
            ..g
        };
        assert_eq!(no_pos.serialize(), "1024 768\n");
        assert_eq!(WindowGeometry::parse(&no_pos.serialize()), Some(no_pos));
    }

    #[test]
    fn garbage_and_partial_lines_are_absent_not_wrong() {
        for text in [
            "",
            "1200",
            "1200 800 5",
            "a b",
            "1200 800 1 2 3",
            "1200 800 x y",
        ] {
            assert_eq!(WindowGeometry::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_stored_extent_is_clamped_and_an_absurd_position_dropped() {
        let g = WindowGeometry::parse("1 99999999 100000 5").expect("parses");
        assert_eq!((g.width, g.height), (MIN_EXTENT, MAX_EXTENT));
        assert_eq!(
            g.position, None,
            "a position off any plausible desktop is dropped"
        );
        let kept = WindowGeometry::parse("800 600 -100 -100").expect("parses");
        assert_eq!(kept.position, Some((-100, -100)));
    }

    #[test]
    fn load_and_save_round_trip_and_a_missing_file_is_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join(FILE_NAME);
        assert_eq!(WindowGeometry::load(&path), None);
        let g = WindowGeometry {
            width: 900,
            height: 700,
            position: Some((10, 20)),
        };
        g.save(&path).expect("save creates the parent");
        assert_eq!(WindowGeometry::load(&path), Some(g));
        assert!(
            !path.with_extension("tmp").exists(),
            "the temporary file is renamed away"
        );
    }

    #[test]
    fn the_config_dir_rule_matches_the_core() {
        let xdg = OsStr::new("/x/cfg");
        let home = OsStr::new("/home/u");
        assert_eq!(
            resolve_config_dir(Some(xdg), Some(home)),
            Some(PathBuf::from("/x/cfg/pmacs"))
        );
        assert_eq!(
            resolve_config_dir(Some(OsStr::new("")), Some(home)),
            Some(PathBuf::from("/home/u/.config/pmacs")),
            "a blank XDG_CONFIG_HOME is absent"
        );
        assert_eq!(
            resolve_config_dir(None, Some(home)),
            Some(PathBuf::from("/home/u/.config/pmacs"))
        );
        assert_eq!(resolve_config_dir(None, None), None);
    }
}

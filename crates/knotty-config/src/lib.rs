//! The configuration file: parsed, validated, defaults merged, and serialized
//! as the one JSON blob that crosses the boundary.
//!
//! Reading the file is here and watching it is the app's, which is the split
//! `01-architecture.md` draws. What the app gets back is one object and never
//! a getter per key, so that a key added later moves this schema and leaves
//! the header alone. cf. `02-ffi.md`

use std::path::Path;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// What a load came to.
///
/// Both halves are always there: a file that will not parse comes back as the
/// defaults with a diagnostic beside them, because a window opens either way
/// and the first load has no previous configuration to keep.
pub struct Loaded {
    /// The configuration as one JSON object.
    pub json: String,
    /// What went wrong, or none when nothing did.
    pub diagnostic: Option<String>,
}

/// Read the configuration at `path`, or the defaults where there is no file.
pub fn load(path: &Path) -> Loaded {
    let (config, diagnostic) = match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => (config, None),
            Err(error) => (Config::default(), Some(error.to_string())),
        },
        // No file is not a failure: it is what every first run has.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
        Err(error) => (
            Config::default(),
            Some(format!("{} could not be read: {error}", path.display())),
        ),
    };

    Loaded {
        // Every value is validated by the time it is here, and none of what
        // serde_json refuses — a non-string key, a float that is not a number
        // — can be reached from this schema.
        json: serde_json::to_string(&config).expect("a validated config serializes"),
        diagnostic,
    }
}

/// Everything v1 opens: eight keys in four groups.
///
/// The schema stands here whole, and the app reads out of the blob whichever
/// part of it that app has a use for yet.
///
/// **An unknown key is a diagnostic and not a shrug.** A misspelled `famly`
/// that parsed would leave the user reading a file that says one thing and a
/// terminal doing another, which is the case the user story is about. The
/// price is that a file written for a later build, with a key this one has
/// not grown yet, is refused whole — paid because the file and the binary
/// travel together on one machine. cf. `05-swift-app.md` 10
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub font: Font,
    pub theme: Theme,
    pub terminal: Terminal,
    pub bell: Bell,
}

/// The face the grid is measured from and drawn in.
#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Font {
    pub family: String,
    #[serde(deserialize_with = "point_size")]
    pub size: f64,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".to_owned(),
            size: 13.0,
        }
    }
}

/// The colours, each of which is absent until the user writes one.
///
/// Absent is a default and not a gap: what stands in for one is the
/// terminal's own colour, which is the VT engine's to say and not this
/// file's to restate.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Theme {
    pub background: Option<Color>,
    pub foreground: Option<Color>,
    pub cursor: Option<Color>,
    pub palette: Option<[Color; 16]>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Terminal {
    #[serde(rename = "option-as-meta")]
    pub option_as_meta: OptionAsMeta,
}

/// Which `⌥` sends Meta rather than making a character.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OptionAsMeta {
    Left,
    Right,
    #[default]
    Both,
    None,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Bell {
    pub mode: BellMode,
}

/// What the bell does. Visual by default: macOS has no sound that would be
/// the obvious one. cf. `05-swift-app.md` 8
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BellMode {
    #[default]
    Visual,
    Sound,
    Bounce,
    Off,
}

/// A colour, written `#rrggbb` and carried across as three numbers so that
/// nobody past this crate parses one again.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text)
            .ok_or_else(|| de::Error::custom(format!("`{text}` is not a colour like `#1e1e1e`")))
    }
}

impl Color {
    fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Self {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        })
    }
}

/// A font size in points. Neither a face of no size nor one of infinite size
/// has a cell to measure, and the grid is measured from the face.
fn point_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let size = f64::deserialize(deserializer)?;
    if size.is_finite() && size > 0.0 {
        Ok(size)
    } else {
        Err(de::Error::custom(format!(
            "`{size}` is not a font size: it must be a number above zero"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::Value;

    use super::*;

    /// Load what `text` says, through the file the app would have written it
    /// to — the missing-file case is the one thing here that is about a path
    /// and not about the text.
    fn loaded(text: &str) -> Loaded {
        let mut file = tempfile::NamedTempFile::new().expect("a temporary file");
        file.write_all(text.as_bytes()).expect("write the config");
        load(file.path())
    }

    fn json(loaded: &Loaded) -> Value {
        serde_json::from_str(&loaded.json).expect("the blob is JSON")
    }

    #[test]
    fn a_font_written_down_comes_back() {
        let loaded = loaded("[font]\nfamily = \"Menlo\"\nsize = 15.5\n");
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(json(&loaded)["font"]["family"], "Menlo");
        assert_eq!(json(&loaded)["font"]["size"], 15.5);
    }

    #[test]
    fn no_file_is_the_defaults_and_not_a_failure() {
        let loaded = load(Path::new("/nonexistent/knotty/config.toml"));
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(json(&loaded)["font"]["family"], "JetBrains Mono");
        assert_eq!(json(&loaded)["font"]["size"], 13.0);
    }

    #[test]
    fn what_the_file_leaves_out_is_filled_in() {
        let loaded = loaded("[font]\nsize = 16.0\n");
        assert_eq!(loaded.diagnostic, None);
        assert_eq!(json(&loaded)["font"]["family"], "JetBrains Mono");
        assert_eq!(json(&loaded)["font"]["size"], 16.0);
        assert_eq!(json(&loaded)["bell"]["mode"], "visual");
        assert_eq!(json(&loaded)["terminal"]["option-as-meta"], "both");
        assert_eq!(json(&loaded)["theme"]["background"], Value::Null);
    }

    #[test]
    fn the_whole_schema_crosses_whether_or_not_this_milestone_reads_it() {
        let loaded = loaded(
            r##"
            [theme]
            background = "#1e1e1e"
            foreground = "#d4d4d4"
            cursor = "#ffffff"
            palette = [
              "#000000", "#cc6666", "#b5bd68", "#f0c674",
              "#81a2be", "#b294bb", "#8abeb7", "#c5c8c6",
              "#666666", "#d54e53", "#b9ca4a", "#e7c547",
              "#7aa6da", "#c397d8", "#70c0b1", "#eaeaea",
            ]

            [terminal]
            option-as-meta = "right"

            [bell]
            mode = "off"
            "##,
        );
        assert_eq!(loaded.diagnostic, None);
        let json = json(&loaded);
        assert_eq!(
            json["theme"]["background"],
            serde_json::json!({"r": 30, "g": 30, "b": 30})
        );
        assert_eq!(json["theme"]["palette"][1]["r"], 204);
        assert_eq!(json["theme"]["palette"].as_array().map(Vec::len), Some(16));
        assert_eq!(json["terminal"]["option-as-meta"], "right");
        assert_eq!(json["bell"]["mode"], "off");
    }

    /// Each of the four ways a value can be wrong: the wrong type, a number
    /// out of range, a colour that is not one, and a key nobody knows.
    #[test]
    fn a_bad_value_is_a_diagnostic_and_the_defaults() {
        for text in [
            "[font]\nsize = \"large\"\n",
            "[font]\nsize = -3.0\n",
            "[theme]\nbackground = \"dark grey\"\n",
            "[font]\nfamly = \"Menlo\"\n",
            "[bell]\nmode = \"flash\"\n",
            "[theme]\npalette = [\"#000000\"]\n",
        ] {
            let loaded = loaded(text);
            assert!(loaded.diagnostic.is_some(), "{text:?} was accepted");
            assert_eq!(
                json(&loaded)["font"]["family"],
                "JetBrains Mono",
                "{text:?} did not fall back to the defaults",
            );
        }
    }
}

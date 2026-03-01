use std::fs;
use std::path::{Path, PathBuf};

use crate::core::glyph::DEFAULT_COLORS;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub font_size: f32,
    pub grid_cols: Option<usize>,
    pub grid_rows: Option<usize>,
    pub palette: [u32; 16],
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            font_size: 32.0,
            grid_cols: None,
            grid_rows: None,
            palette: DEFAULT_COLORS,
        }
    }
}

impl AppConfig {
    pub fn load_or_create(path: &Path) -> Self {
        if let Ok(contents) = fs::read_to_string(path) {
            let cfg = Self::from_ini(&contents);
            if cfg.is_some() {
                return cfg.unwrap();
            }
        }

        let cfg = Self::default();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, cfg.to_ini());
        cfg
    }

    fn from_ini(contents: &str) -> Option<Self> {
        let mut cfg = Self::default();
        let mut section = String::new();

        for raw_line in contents.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();

            match (section.as_str(), key.as_str()) {
                ("font", "size") => {
                    if let Ok(v) = value.parse::<f32>() {
                        if v >= 8.0 && v <= 96.0 {
                            cfg.font_size = v;
                        }
                    }
                }
                ("grid", "cols") => {
                    if let Ok(v) = value.parse::<usize>() {
                        cfg.grid_cols = if v > 0 { Some(v) } else { None };
                    }
                }
                ("grid", "rows") => {
                    if let Ok(v) = value.parse::<usize>() {
                        cfg.grid_rows = if v > 0 { Some(v) } else { None };
                    }
                }
                ("colors", "palette") => {
                    if let Some(palette) = parse_palette(value) {
                        cfg.palette = palette;
                    }
                }
                _ => {}
            }
        }

        Some(cfg)
    }

    fn to_ini(&self) -> String {
        let mut out = String::new();
        out.push_str("# gui-engine config\n\n");
        out.push_str("[font]\n");
        out.push_str(&format!("size = {}\n\n", self.font_size));
        out.push_str("[grid]\n");
        out.push_str(&format!(
            "cols = {}\nrows = {}\n\n",
            self.grid_cols.unwrap_or(0),
            self.grid_rows.unwrap_or(0)
        ));
        out.push_str("[colors]\n");
        out.push_str("palette = ");
        for (i, c) in self.palette.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("#{:06x}", c));
        }
        out.push('\n');
        out
    }
}

fn parse_palette(value: &str) -> Option<[u32; 16]> {
    let parts: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
    if parts.len() != 16 {
        return None;
    }

    let mut palette = [0u32; 16];
    for (i, part) in parts.iter().enumerate() {
        let p = part.trim_start_matches('#').trim_start_matches("0x");
        if p.len() != 6 {
            return None;
        }
        if let Ok(v) = u32::from_str_radix(p, 16) {
            palette[i] = v;
        } else {
            return None;
        }
    }

    Some(palette)
}

pub fn config_path(base: &Path) -> PathBuf {
    base.join("gui-engine.ini")
}

pub struct GStreamerUtils;

impl GStreamerUtils {
    pub fn hex_to_rgb(hex: &str) -> Result<(f32, f32, f32), String> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err("Invalid hex color format".to_string());
        }

        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| "Invalid hex color")?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| "Invalid hex color")?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| "Invalid hex color")?;

        Ok((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0))
    }
}

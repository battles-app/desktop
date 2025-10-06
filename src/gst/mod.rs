// GStreamer utilities and types for the compositor

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Preview,
    VirtualCamera,
    NDI,
    RTMP { url: String },
    WebRTC,
    File { path: String },
}

pub struct GStreamerUtils;

impl GStreamerUtils {
    pub fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), String> {
        if !hex.starts_with('#') || hex.len() != 7 {
            return Err("Invalid hex color format. Expected #RRGGBB".to_string());
        }

        let r = u8::from_str_radix(&hex[1..3], 16)
            .map_err(|_| "Invalid red component")?;
        let g = u8::from_str_radix(&hex[3..5], 16)
            .map_err(|_| "Invalid green component")?;
        let b = u8::from_str_radix(&hex[5..7], 16)
            .map_err(|_| "Invalid blue component")?;

        Ok((r, g, b))
    }
}

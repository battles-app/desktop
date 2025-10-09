use elgato_streamdeck::{new_hidapi, list_devices, StreamDeck, info::Kind};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use parking_lot::Mutex;
use std::sync::Arc;
use image::RgbaImage;
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;
use ab_glyph::{FontRef, PxScale};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxButton {
    pub id: String,
    pub name: String,
    pub image_url: Option<String>,
    pub is_global: bool, // true for battle board, false for user FX
    pub position: usize, // Original position in the list
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ButtonState {
    pub is_playing: bool,
    pub button: Option<FxButton>,
}

pub struct StreamDeckManager {
    device: Option<StreamDeck>,
    button_states: HashMap<u8, ButtonState>,
    button_layout: Vec<Option<FxButton>>, // Maps Stream Deck button index to FX button
    device_kind: Option<Kind>,
    is_connected: bool,
}

impl StreamDeckManager {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            device: None,
            button_states: HashMap::new(),
            button_layout: Vec::new(),
            device_kind: None,
            is_connected: false,
        })
    }
    
    /// Scan for connected Stream Deck devices
    pub fn scan_devices(&mut self) -> Result<Vec<(Kind, String)>, String> {
        let hid = new_hidapi().map_err(|e| format!("Failed to initialize HidApi: {}", e))?;
        let devices = list_devices(&hid);
        Ok(devices)
    }
    
    /// Connect to the first available Stream Deck device
    pub fn connect(&mut self) -> Result<String, String> {
        let hid = new_hidapi().map_err(|e| format!("Failed to initialize HidApi: {}", e))?;
        let devices = list_devices(&hid);
        
        if devices.is_empty() {
            return Err("No Stream Deck devices found".to_string());
        }
        
        let (kind, serial) = &devices[0];
        
        let device = StreamDeck::connect(&hid, *kind, serial)
            .map_err(|e| format!("Failed to connect to Stream Deck: {}", e))?;
        
        self.device_kind = Some(*kind);
        
        // Get device info
        let serial_number = device.serial_number()
            .unwrap_or_else(|_| "Unknown".to_string());
        
        let info = format!(
            "Connected to {:?} (Serial: {})",
            kind,
            serial_number
        );
        
        // Set initial brightness
        let _ = device.set_brightness(50);
        
        // Clear all buttons
        for i in 0..self.button_count() as u8 {
            let size = self.get_button_size();
            let img = image::RgbaImage::new(size, size);
            let dynamic_img = image::DynamicImage::ImageRgba8(img);
            let _ = device.set_button_image(i, dynamic_img);
        }
        let _ = device.flush();
        
        self.device = Some(device);
        self.is_connected = true;
        
        Ok(info)
    }
    
    /// Disconnect from Stream Deck
    pub fn disconnect(&mut self) {
        println!("[Stream Deck] Disconnecting and cleaning up...");
        
        // Clear all buttons before disconnect
        if self.device.is_some() {
            let _ = self.clear_all_buttons();
        }
        
        if let Some(device) = self.device.take() {
            let _ = device.reset();
        }
        
        self.is_connected = false;
        self.button_states.clear();
        self.button_layout.clear();
        
        println!("[Stream Deck] ✅ Disconnected and cleaned up");
    }
    
    /// Check if device is connected
    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
    
    /// Get device kind name
    pub fn device_kind_name(&self) -> String {
        match self.device_kind {
            Some(kind) => format!("{:?}", kind),
            None => "Unknown".to_string(),
        }
    }
    
    /// Get serial number
    pub fn get_serial_number(&self) -> Result<String, String> {
        if let Some(ref device) = self.device {
            device.serial_number()
                .map_err(|e| format!("Failed to get serial: {}", e))
        } else {
            Err("No device connected".to_string())
        }
    }
    
    /// Get number of buttons on the device
    pub fn button_count(&self) -> usize {
        match self.device_kind {
            Some(Kind::Original) => 15,
            Some(Kind::OriginalV2) => 15,
            Some(Kind::Mk2) => 15,
            Some(Kind::Mk2Scissor) => 15,
            Some(Kind::Mini) => 6,
            Some(Kind::MiniMk2) => 6,
            Some(Kind::Xl) => 32,
            Some(Kind::XlV2) => 32,
            Some(Kind::Plus) => 8,
            Some(Kind::Neo) => 8,
            Some(Kind::Pedal) => 3,
            None => 0,
        }
    }
    
    /// Get button size for the device
    /// Using high-DPI sizes per Elgato documentation: 144x144 for standard, 96x96 for XL
    fn get_button_size(&self) -> u32 {
        match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) |
            Some(Kind::Mk2) | Some(Kind::Mk2Scissor) | 
            Some(Kind::Mini) | Some(Kind::MiniMk2) => 144, // High-DPI: 144x144
            Some(Kind::Xl) | Some(Kind::XlV2) => 96, // XL uses 96x96
            Some(Kind::Plus) | Some(Kind::Neo) => 200,
            Some(Kind::Pedal) => 0,
            None => 144,
        }
    }
    
    /// Update button layout with FX buttons
    /// Battle board effects go on left side, user FX on right side
    /// Find cached image from frontend cache (NO downloading - images are pre-cached by frontend!)
    /// Cache files are named after the FX name, e.g., "x2.jpg", "galaxy-001.mp4", "10 sec countdown_1.mp4"
    fn find_cached_image(&self, fx_name: &str) -> Option<PathBuf> {
        let cache_dir = std::env::temp_dir().join("battles_fx_cache");
        
        if !cache_dir.exists() {
            return None;
        }
        
        // Normalize FX name for matching (lowercase, remove spaces)
        let normalized_name = fx_name.to_lowercase().replace(" ", " ");
        
        // Try to find any cached file that matches the FX name
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                
                // Check if filename starts with or contains the FX name
                if let Some(filename) = path.file_name() {
                    let filename_str = filename.to_string_lossy().to_lowercase();
                    
                    // Check if filename contains the normalized FX name
                    if filename_str.contains(&normalized_name) || filename_str.starts_with(&normalized_name) {
                        // Must be an image file (not video)
                        if let Some(ext) = path.extension() {
                            let ext_str = ext.to_string_lossy().to_lowercase();
                            if ext_str == "jpg" || ext_str == "jpeg" || ext_str == "png" || ext_str == "webp" || ext_str == "gif" {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        
        // Also try direct patterns with the FX name
        let possible_patterns = vec![
            format!("{}.jpg", fx_name),
            format!("{}.jpeg", fx_name),
            format!("{}.png", fx_name),
            format!("{}.webp", fx_name),
        ];
        
        for pattern in possible_patterns {
            let path = cache_dir.join(&pattern);
            if path.exists() {
                return Some(path);
            }
        }
        
        None
    }
    
    /// Download image from Nuxt proxy, cache it, and trigger re-render
    fn download_image_to_cache(&self, fx_button: &FxButton) {
        if fx_button.image_url.is_none() {
            return;
        }
        
        let cache_dir = std::env::temp_dir().join("battles_fx_cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        
        // Cache filename: {name}.jpg (e.g., "x2.jpg", "galaxy.jpg")
        let cache_filename = format!("{}.jpg", fx_button.name);
        let cache_path = cache_dir.join(&cache_filename);
        
        // Skip if already cached
        if cache_path.exists() {
            return;
        }
        
        // Download from Nuxt proxy (non-blocking in background)
        let image_url = fx_button.image_url.clone().unwrap();
        let name = fx_button.name.clone();
        let fx_id = fx_button.id.clone();
        let cache_path_clone = cache_path.clone();
        
        std::thread::spawn(move || {
            let full_url = format!("https://local.battles.app:3000{}", image_url);
            
            match reqwest::blocking::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(10))
                .build()
            {
                Ok(client) => {
                    match client.get(&full_url).send() {
                        Ok(response) if response.status().is_success() => {
                            match response.bytes() {
                                Ok(bytes) => {
                                    if let Err(e) = std::fs::write(&cache_path_clone, &bytes) {
                                        println!("[Stream Deck] ⚠️ Failed to cache image for {}: {}", name, e);
                                    } else {
                                        println!("[Stream Deck] ✅ Cached image for {}: {:?}", name, cache_path_clone.file_name());
                                        
                                        // Trigger re-render of this specific button
                                        let mut manager_lock = STREAMDECK_MANAGER.lock();
                                        if let Some(ref mut manager) = *manager_lock {
                                            // Find and re-render the button
                                            let _ = manager.refresh_button_by_id(&fx_id);
                                        }
                                    }
                                }
                                Err(e) => println!("[Stream Deck] ⚠️ Failed to read image for {}: {}", name, e),
                            }
                        }
                        Ok(response) => println!("[Stream Deck] ⚠️ HTTP {} for {}", response.status(), name),
                        Err(e) => println!("[Stream Deck] ⚠️ Download failed for {}: {}", name, e),
                    }
                }
                Err(e) => println!("[Stream Deck] ⚠️ Failed to create HTTP client for {}: {}", name, e),
            }
        });
    }
    
    pub fn update_layout(&mut self, battle_board: Vec<FxButton>, user_fx: Vec<FxButton>) -> Result<(), String> {
        let button_count = self.button_count();
        if button_count == 0 {
            return Err("No device connected".to_string());
        }
        
        println!("[Stream Deck] Updating layout with {} battle board + {} user FX items", battle_board.len(), user_fx.len());
        
        // Clear all buttons first before mounting new layout
        println!("[Stream Deck] Clearing all buttons before mounting...");
        self.clear_all_buttons()?;
        
        // Start downloading images in background (non-blocking)
        // They will trigger re-renders when complete
        for fx in battle_board.iter().chain(user_fx.iter()) {
            self.download_image_to_cache(fx);
        }
        
        // Initialize layout with None
        self.button_layout = vec![None; button_count];
        
        // Get device dimensions for layout calculation
        let (cols, rows) = match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) | Some(Kind::Mk2) | Some(Kind::Mk2Scissor) => (5, 3),
            Some(Kind::Mini) | Some(Kind::MiniMk2) => (3, 2),
            Some(Kind::Xl) | Some(Kind::XlV2) => (8, 4),
            Some(Kind::Plus) => (4, 2), // 8 buttons in 4x2 grid
            Some(Kind::Neo) => (4, 2), // 8 buttons
            Some(Kind::Pedal) => (3, 1), // 3 pedals
            None => return Err("Unknown device type".to_string()),
        };
        
        // For XL devices: Reserve rightmost column for control buttons
        // Layout: [Battle Board (left 5 cols)] [User FX (2 cols, max 12)] [Controls (right col)]
        if matches!(self.device_kind, Some(Kind::Xl) | Some(Kind::XlV2)) {
            // Place battle board on left (columns 0-4, max 20 buttons)
            let mut battle_index = 0;
            for row in 0..rows {
                for col in 0..5 {
                    if battle_index < battle_board.len() {
                        let button_idx = row * cols + col;
                        if button_idx < button_count {
                            self.button_layout[button_idx] = Some(battle_board[battle_index].clone());
                            battle_index += 1;
                        }
                    }
                }
            }
            
            // Place user FX in columns 5-6 (max 8 buttons, but limit to 12 total across both columns)
            let mut user_index = 0;
            for row in 0..rows {
                for col in 5..7 {
                    if user_index < user_fx.len() && user_index < 12 {
                        let button_idx = row * cols + col;
                        if button_idx < button_count {
                            self.button_layout[button_idx] = Some(user_fx[user_index].clone());
                            user_index += 1;
                        }
                    }
                }
            }
            
            // Place control buttons in rightmost column (column 7)
            let control_buttons = vec![
                ("INTRO", [138, 43, 226]), // Purple
                ("PARTY", [255, 105, 180]), // Hot pink
                ("BREAK", [30, 144, 255]),  // Blue
                ("END", [220, 20, 60]),     // Crimson
            ];
            
            for (row, (name, _color)) in control_buttons.iter().enumerate() {
                let button_idx = row * cols + 7; // Column 7 (rightmost)
                if button_idx < button_count {
                    self.button_layout[button_idx] = Some(FxButton {
                        id: format!("control_{}", name.to_lowercase()),
                        name: name.to_string(),
                        image_url: None,
                        is_global: false,
                        position: row,
                    });
                }
            }
        } else {
            // Standard layout for smaller devices (left = battle board, right = user FX)
            let mid_col = cols / 2;
            
            // Place battle board effects on left side
            let mut battle_index = 0;
            for row in 0..rows {
                for col in 0..mid_col {
                    if battle_index < battle_board.len() {
                        let button_idx = row * cols + col;
                        if button_idx < button_count {
                            self.button_layout[button_idx] = Some(battle_board[battle_index].clone());
                            battle_index += 1;
                        }
                    }
                }
            }
            
            // Place user FX on right side
            let mut user_index = 0;
            for row in 0..rows {
                for col in mid_col..cols {
                    if user_index < user_fx.len() {
                        let button_idx = row * cols + col;
                        if button_idx < button_count {
                            self.button_layout[button_idx] = Some(user_fx[user_index].clone());
                            user_index += 1;
                        }
                    }
                }
            }
        }
        
        // Render all buttons
        self.render_all_buttons()?;
        
        Ok(())
    }
    
    /// Render all buttons based on current layout
    fn render_all_buttons(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            return Ok(());
        }
        
        // Collect all button images first
        let mut button_images: Vec<(u8, Option<image::DynamicImage>)> = Vec::new();
        
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            let image = if let Some(button) = button_opt {
                let is_playing = self.button_states
                    .get(&(idx as u8))
                    .map(|s| s.is_playing)
                    .unwrap_or(false);
                Some(self.create_button_image(button, is_playing)?)
            } else {
                // Empty button
                let size = self.get_button_size();
                let img = image::RgbaImage::new(size, size);
                Some(image::DynamicImage::ImageRgba8(img))
            };
            button_images.push((idx as u8, image));
        }
        
        // Now set all button images
        if let Some(ref mut device) = self.device {
            for (idx, image_opt) in button_images {
                if let Some(image) = image_opt {
                    device.set_button_image(idx, image)
                        .map_err(|e| format!("Failed to set button image: {}", e))?;
                }
            }
            
            // Flush changes to device
            device.flush().map_err(|e| format!("Failed to flush device: {}", e))?;
        }
        
        Ok(())
    }
    
    /// Create button image with text and styling
    fn create_button_image(&self, fx_button: &FxButton, is_playing: bool) -> Result<image::DynamicImage, String> {
        // Get button size
        let size = self.get_button_size();
        
        // Try to load cached image from frontend cache (NO downloading!)
        // Cache files are named after the FX name, e.g., "x2.jpg", "galaxy.mp4"
        let cached_image = if let Some(cached_path) = self.find_cached_image(&fx_button.name) {
            println!("[Stream Deck] ✅ Found cached image for {}: {:?}", fx_button.name, cached_path.file_name());
            image::open(&cached_path).ok()
        } else {
            None
        };
        
        // Create base button image
        let mut img = if let Some(cached_img) = cached_image {
            // Use cached image, resize to fit button
            let resized = cached_img.resize_exact(size, size, image::imageops::FilterType::Triangle);
            resized.to_rgba8()
        } else {
            // Fall back to colored background if no image
            let mut img = RgbaImage::new(size, size);
            
            // Check if this is a control button
            let bg_color = if fx_button.id.starts_with("control_") {
                // Control buttons have specific colors
                match fx_button.name.as_str() {
                    "INTRO" => image::Rgba([138, 43, 226, 255]), // Purple
                    "PARTY" => image::Rgba([255, 105, 180, 255]), // Hot pink
                    "BREAK" => image::Rgba([30, 144, 255, 255]),  // Blue
                    "END" => image::Rgba([220, 20, 60, 255]),     // Crimson
                    _ => image::Rgba([80, 80, 80, 255]), // Gray fallback
                }
            } else if is_playing {
                image::Rgba([50, 205, 50, 255]) // Green when playing
            } else if fx_button.is_global {
                image::Rgba([138, 43, 226, 255]) // Purple for battle board
            } else {
                image::Rgba([30, 144, 255, 255]) // Blue for user FX
            };
            
            draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), bg_color);
            img
        };
        
        // Add colored border overlay for playing state or type indicator
        // Skip borders for control buttons
        if !fx_button.id.starts_with("control_") {
            if is_playing {
                // Bright green border when playing
                let border_color = image::Rgba([50, 255, 50, 255]);
                for i in 0..6 {
                    let rect = Rect::at(i, i).of_size(size - (i * 2) as u32, size - (i * 2) as u32);
                    imageproc::drawing::draw_hollow_rect_mut(&mut img, rect, border_color);
                }
            } else {
                // Subtle colored border to indicate type
                let border_color = if fx_button.is_global {
                    image::Rgba([138, 43, 226, 180]) // Purple tint for battle board
                } else {
                    image::Rgba([30, 144, 255, 180]) // Blue tint for user FX
                };
                for i in 0..3 {
                    let rect = Rect::at(i, i).of_size(size - (i * 2) as u32, size - (i * 2) as u32);
                    imageproc::drawing::draw_hollow_rect_mut(&mut img, rect, border_color);
                }
            }
        }
        
        // Render text with FX name
        let font_data = include_bytes!("../assets/DejaVuSans.ttf");
        let font = FontRef::try_from_slice(font_data)
            .map_err(|e| format!("Failed to load font: {:?}", e))?;
        
        // Control buttons get larger centered text, FX buttons get bottom text bar
        if fx_button.id.starts_with("control_") {
            // Large centered text for control buttons
            let font_scale = PxScale::from((size as f32 * 0.18).max(14.0));
            let display_name = fx_button.name.clone();
            let text_color = image::Rgba([255, 255, 255, 255]);
            
            // Center text both horizontally and vertically
            let text_x = ((size as f32 - (display_name.len() as f32 * font_scale.x * 0.5)) / 2.0) as i32;
            let text_y = ((size as f32 - font_scale.y) / 2.0) as i32;
            
            draw_text_mut(&mut img, text_color, text_x, text_y, font_scale, &font, &display_name);
        } else {
            // Add semi-transparent text background at bottom for FX buttons
            let text_bg_height = (size as f32 * 0.25) as u32;
            let text_bg_y = size - text_bg_height;
            draw_filled_rect_mut(
                &mut img,
                Rect::at(0, text_bg_y as i32).of_size(size, text_bg_height),
                image::Rgba([0, 0, 0, 180])
            );
            
            // Calculate font size based on button size
            let font_scale = PxScale::from((size as f32 * 0.13).max(10.0));
            
            // Prepare text (truncate if too long)
            let display_name = if fx_button.name.len() > 10 {
                format!("{}...", &fx_button.name[..7])
            } else {
                fx_button.name.clone()
            };
            
            // Position text in the text background area (bottom of button)
            let text_color = image::Rgba([255, 255, 255, 255]); // White text
            let text_x = ((size as f32 - (display_name.len() as f32 * font_scale.x * 0.5)) / 2.0) as i32;
            let text_y = (text_bg_y + (text_bg_height / 2) - (font_scale.y as u32 / 2)) as i32;
            
            // Draw text (no shadow needed on dark background)
            draw_text_mut(&mut img, text_color, text_x, text_y, font_scale, &font, &display_name);
        }
        
        Ok(image::DynamicImage::ImageRgba8(img))
    }
    
    /// Clear all buttons
    pub fn clear_all_buttons(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            return Ok(());
        }
        
        let size = self.get_button_size();
        let button_count = self.button_count();
        
        // Create empty images for all buttons
        let mut button_images: Vec<(u8, image::DynamicImage)> = Vec::new();
        for i in 0..button_count as u8 {
            let img = image::RgbaImage::new(size, size);
            button_images.push((i, image::DynamicImage::ImageRgba8(img)));
        }
        
        // Now set all buttons
        if let Some(ref mut device) = self.device {
            for (idx, image) in button_images {
                device.set_button_image(idx, image)
                    .map_err(|e| format!("Failed to clear button: {}", e))?;
            }
            
            device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
        }
        
        Ok(())
    }
    
    /// Get button state changes (returns list of pressed buttons since last check)
    pub fn get_button_changes(&mut self) -> Result<Vec<(u8, bool)>, String> {
        // Note: The elgato-streamdeck library doesn't have async button reading
        // We'll need to implement a polling mechanism in the frontend
        // For now, return an empty list
        Ok(vec![])
    }
    
    /// Handle button press (toggle play/stop)
    pub fn handle_button_press(&mut self, button_idx: u8) -> Option<(String, bool)> {
        // Get the FX button at this position
        let fx_button = self.button_layout.get(button_idx as usize)?.as_ref()?.clone();
        
        // Toggle playing state
        let entry = self.button_states
            .entry(button_idx)
            .or_insert(ButtonState {
                is_playing: false,
                button: Some(fx_button.clone()),
            });
        
        entry.is_playing = !entry.is_playing;
        let new_state = entry.is_playing;
        let fx_id = fx_button.id.clone();
        
        // Update button visual
        if self.device.is_some() {
            let is_playing = new_state;
            if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                if let Some(ref mut device) = self.device {
                    let _ = device.set_button_image(button_idx, image);
                    let _ = device.flush();
                }
            }
        }
        
        // Return (fx_id, is_playing)
        Some((fx_id, new_state))
    }
    
    /// Refresh a single button by FX ID (called after image downloads)
    pub fn refresh_button_by_id(&mut self, fx_id: &str) -> Result<(), String> {
        // Find button with this FX ID
        let mut button_to_update: Option<(u8, FxButton)> = None;
        
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            if let Some(fx_button) = button_opt {
                if fx_button.id == fx_id {
                    button_to_update = Some((idx as u8, fx_button.clone()));
                    break;
                }
            }
        }
        
        if let Some((idx, fx_button)) = button_to_update {
            let is_playing = self.button_states
                .get(&idx)
                .map(|s| s.is_playing)
                .unwrap_or(false);
            
            // Re-render button with cached image
            if self.device.is_some() {
                if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                    if let Some(ref mut device) = self.device {
                        device.set_button_image(idx, image)
                            .map_err(|e| format!("Failed to set button image: {}", e))?;
                        device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
                        println!("[Stream Deck] 🔄 Refreshed button {} ({}) with cached image", idx, fx_button.name);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Update button state (called when FX stops playing)
    pub fn set_button_state(&mut self, fx_id: &str, is_playing: bool) -> Result<(), String> {
        // Find button with this FX ID and update state
        let mut button_to_update: Option<(u8, FxButton)> = None;
        
        for (idx, button_opt) in self.button_layout.iter().enumerate() {
            if let Some(fx_button) = button_opt {
                if fx_button.id == fx_id {
                    button_to_update = Some((idx as u8, fx_button.clone()));
                    break;
                }
            }
        }
        
        if let Some((idx, fx_button)) = button_to_update {
            // Update state
            self.button_states
                .entry(idx)
                .and_modify(|s| s.is_playing = is_playing)
                .or_insert(ButtonState {
                    is_playing,
                    button: Some(fx_button.clone()),
                });
            
            // Update visual
            if self.device.is_some() {
                if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                    if let Some(ref mut device) = self.device {
                        device.set_button_image(idx, image)
                            .map_err(|e| format!("Failed to set button image: {}", e))?;
                        device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
                    }
                }
            }
        }
        
        Ok(())
    }
}

// Global Stream Deck manager instance
lazy_static::lazy_static! {
    pub static ref STREAMDECK_MANAGER: Arc<Mutex<Option<StreamDeckManager>>> = Arc::new(Mutex::new(None));
}


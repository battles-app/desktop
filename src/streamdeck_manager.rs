use elgato_streamdeck::{new_hidapi, list_devices, StreamDeck, info::Kind};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use parking_lot::Mutex;
use std::sync::Arc;
use image::{RgbaImage, Rgba};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut, draw_filled_circle_mut};
use imageproc::rect::Rect;
use ab_glyph::{FontRef, PxScale};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ButtonType {
    FxButton(FxButton),
    TvMonitorControl,
    VideoToggle,
    VideoLoopUp,
    VideoLoopDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ButtonPressEvent {
    FxPressed { fx_id: String, is_playing: bool },
    TvMonitorToggle,
    VideoToggle { is_playing: bool },
    VideoLoopBrowse { direction: i32, loop_index: usize }, // direction: 1 for up, -1 for down
}

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
    button_layout: Vec<Option<ButtonType>>, // Maps Stream Deck button index to button type
    device_kind: Option<Kind>,
    is_connected: bool,
    loading_animation_active: bool,
    image_cache: HashMap<String, PathBuf>, // Cache of FX name -> image path to avoid repeated filesystem searches
    current_video_loop_index: usize, // Current video loop being previewed
    last_button_states: HashMap<u8, bool>, // Track physical button states for debouncing
    last_event_time: HashMap<u8, std::time::Instant>, // Track last event time per button for deduplication
}

impl StreamDeckManager {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            device: None,
            button_states: HashMap::new(),
            button_layout: Vec::new(),
            device_kind: None,
            is_connected: false,
            loading_animation_active: false,
            image_cache: HashMap::new(),
            current_video_loop_index: 0,
            last_button_states: HashMap::new(),
            last_event_time: HashMap::new(),
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
        
        self.device = Some(device);
        self.is_connected = true;
        
        // Start animation ONLY if no buttons are loaded (first connect)
        // Check for ACTUAL button content, not just vector emptiness (fixes race condition with update_layout)
        let button_layout_size = self.button_layout.len();
        let has_buttons = self.button_layout.iter().any(|b| b.is_some());
        crate::file_logger::log(&format!("[StreamDeck] connect() called - button_layout.len()={}, has_buttons={}", 
            button_layout_size, has_buttons));
        
        if !has_buttons {
            // No buttons loaded yet - start animation
            self.loading_animation_active = true;
            crate::file_logger::log("[StreamDeck] ▶️ Animation started (no buttons loaded yet)");
        } else {
            // Buttons already loaded - don't restart animation
            crate::file_logger::log(&format!("[StreamDeck] ℹ️ Buttons already loaded ({}), NOT starting animation", 
                button_layout_size));
        }
        
        // DON'T play animation here - it blocks everything!
        // Background thread will handle ALL animation (including initial reveal)
        // This allows frontend to load data in PARALLEL with animation

        Ok(info)
    }
    
    /// Disconnect from Stream Deck
    pub fn disconnect(&mut self) {
        crate::file_logger::log("[Stream Deck] Disconnecting and cleaning up...");
        
        // Clear all buttons before disconnect
        if self.device.is_some() {
            let _ = self.clear_all_buttons();
        }
        
        if let Some(device) = self.device.take() {
            let _ = device.reset();
        }
        
        self.is_connected = false;
        self.button_states.clear();
        self.last_button_states.clear(); // Clear debounce state
        self.last_event_time.clear(); // Clear event deduplication state
        
        // DON'T clear button_layout - preserve it for reconnection!
        // This prevents animation from restarting when device reconnects
        crate::file_logger::log(&format!("[Stream Deck] ✅ Disconnected (preserved {} button layouts for reconnect)", 
            self.button_layout.len()));
    }
    
    /// Check if device is connected
    pub fn is_connected(&self) -> bool {
        self.is_connected
    }
    
    /// Get count of loaded buttons (for idempotency check)
    pub fn get_loaded_button_count(&self) -> usize {
        self.button_layout.iter().filter(|b| b.is_some()).count()
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
    
    /// Convert HSV to RGB (for smooth gradient animations)
    #[allow(dead_code)]
    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
        let c = v * s;
        let h_prime = h / 60.0;
        let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
        let m = v - c;
        
        let (r, g, b) = if h_prime < 1.0 {
            (c, x, 0.0)
        } else if h_prime < 2.0 {
            (x, c, 0.0)
        } else if h_prime < 3.0 {
            (0.0, c, x)
        } else if h_prime < 4.0 {
            (0.0, x, c)
        } else if h_prime < 5.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };
        
        (
            ((r + m) * 255.0) as u8,
            ((g + m) * 255.0) as u8,
            ((b + m) * 255.0) as u8,
        )
    }
    
    /// Create logo button image using favicon.png
    #[allow(dead_code)]
    fn create_logo_button(&self, frame: usize) -> Result<image::DynamicImage, String> {
        let size = self.get_button_size();
        let mut img = RgbaImage::new(size, size);
        
        // Dark gradient background (matches animation)
        let wave_offset = frame as f32 * 8.0;
        let hue = wave_offset % 360.0;
        let (r, g, b) = Self::hsv_to_rgb(hue, 0.3, 0.2);
        draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), Rgba([r, g, b, 255]));
        
        // Load and overlay favicon.png (transparent logo)
        let favicon_bytes = include_bytes!("../favicon.png");
        
        if let Ok(favicon) = image::load_from_memory(favicon_bytes) {
            // Resize favicon to 70% of button size (centered with padding)
            let logo_size = (size as f32 * 0.7) as u32;
            let resized_logo = image::imageops::resize(
                &favicon.to_rgba8(),
                logo_size,
                logo_size,
                image::imageops::FilterType::Lanczos3
            );
            
            // Center the logo on the button
            let offset_x = ((size - logo_size) / 2) as i32;
            let offset_y = ((size - logo_size) / 2) as i32;
            
            // Overlay with alpha blending
            image::imageops::overlay(&mut img, &resized_logo, offset_x as i64, offset_y as i64);
        }
        
        Ok(image::DynamicImage::ImageRgba8(img))
    }
    
    /// Check if loading animation should continue
    pub fn is_loading_animation_active(&self) -> bool {
        self.loading_animation_active
    }
    
    /// Stop loading animation (called when FX buttons are loaded)
    pub fn stop_loading_animation(&mut self) {
        let was_active = self.loading_animation_active;
        self.loading_animation_active = false;
        
        if was_active {
            let button_count = self.button_layout.iter().filter(|b| b.is_some()).count();
            crate::file_logger::log(&format!("[StreamDeck] 🛑 STOPPING animation NOW (was active, {} buttons loaded)", 
                button_count));
        }
    }
    
    /// Play a loading animation with dark gradient wave and brand-colored text
    /// Shows "BATTLES" first, then "LOADING", with continuous background until loaded
    #[allow(dead_code)]
    pub fn play_loading_animation(&mut self) -> Result<(), String> {
        if self.device.is_none() {
            return Err("No device connected".to_string());
        }
        
        let size = self.get_button_size();
        let button_count = self.button_count();
        
        // Get grid dimensions
        let (cols, _rows) = match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) | Some(Kind::Mk2) | Some(Kind::Mk2Scissor) => (5, 3),
            Some(Kind::Mini) | Some(Kind::MiniMk2) => (3, 2),
            Some(Kind::Xl) | Some(Kind::XlV2) => (8, 4),
            Some(Kind::Plus) | Some(Kind::Neo) => (4, 2),
            Some(Kind::Pedal) => (3, 1),
            None => return Err("Unknown device type".to_string()),
        };
        
        let text_battles = "BATTLES";
        let text_loading = "LOADING";
        
        // Logo colors: Pink (#ee2b63), White (#fff), Yellow (#e9b320)
        let logo_colors = [
            Rgba([238, 43, 99, 255]),   // Pink/Red
            Rgba([255, 255, 255, 255]), // White
            Rgba([233, 179, 32, 255]),  // Yellow
        ];
        
        let font_data = include_bytes!("../assets/DejaVuSans.ttf");
        let font = FontRef::try_from_slice(font_data)
            .map_err(|e| format!("Failed to load font: {:?}", e))?;
        
        // Each letter appears every 1 frame (6ms at 166 FPS) - BLAZING FAST!
        let frames_per_letter = 1;
        
        // Phase 1: Show logo + "BATTLES" (1 frame per letter)
        let battles_frames = text_battles.len() * frames_per_letter;
        // Phase 2: Show logo + "LOADING" (1 frame per letter)
        let loading_frames = text_loading.len() * frames_per_letter;
        // Phase 3: Hold both visible (25 frames = ~150ms at 166 FPS)
        let hold_frames = 25;
        
        let total_frames = battles_frames + loading_frames + hold_frames;
        
        for frame in 0..total_frames {
            let mut images = Vec::new();
            
            // Calculate which phase we're in
            let battles_visible = if frame < battles_frames {
                (frame / frames_per_letter).min(text_battles.len())
            } else {
                text_battles.len()
            };
            
            let loading_visible = if frame >= battles_frames {
                ((frame - battles_frames) / frames_per_letter).min(text_loading.len())
            } else {
                0
            };
            
            for button_idx in 0..button_count {
                let row = button_idx / cols;
                let col = button_idx % cols;
                
                // LOGO BUTTON: First key of second row (row 1, col 0)
                let logo_button_idx = cols; // Row 1, Col 0 = 1 * cols + 0
                if button_idx == logo_button_idx {
                    if let Ok(logo_img) = self.create_logo_button(frame) {
                        images.push(logo_img);
                        continue;
                    }
                }
                
                // Create DARK gradient background (matching app's dark theme)
                let mut img = RgbaImage::new(size, size);
                
                // Animated dark gradient wave (slower, more subtle)
                let wave_offset = frame as f32 * 8.0;
                let position_factor = (col as f32 + row as f32) * 25.0;
                let hue = (position_factor + wave_offset) % 360.0;
                
                // Dark gradient: low saturation, low value for dark background
                let (r, g, b) = Self::hsv_to_rgb(hue, 0.3, 0.2);
                let bg_color = Rgba([r, g, b, 255]);
                
                draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), bg_color);
                
                // Draw "BATTLES" on row 1 (starting from col 1, after logo at col 0)
                // Adjust column index for text (shift by 1 to account for logo)
                let text_col = if row == 1 && col > 0 { col - 1 } else { col };
                let should_show_battles = row == 1 && col > 0 && text_col < text_battles.len() && text_col < battles_visible;
                
                // Draw "LOADING" on row 2 (with logo colors)
                let should_show_loading = row == 2 && col < text_loading.len() && col < loading_visible;
                
                if should_show_battles || should_show_loading {
                    let (letter, color_idx) = if should_show_battles {
                        (text_battles.chars().nth(text_col).unwrap(), text_col % logo_colors.len())
                    } else {
                        (text_loading.chars().nth(col).unwrap(), col % logo_colors.len())
                    };
                    
                    // LARGE, BOLD, CENTERED text
                    let scale = PxScale::from((size as f32 * 0.65).max(50.0));
                    let letter_str = letter.to_string();
                    let text_color = logo_colors[color_idx];
                    
                    // Calculate center position for the letter
                    // Approximate letter width (rough estimate for centering)
                    let estimated_letter_width = scale.x * 0.6;
                    let estimated_letter_height = scale.y;
                    
                    let text_x = ((size as f32 - estimated_letter_width) / 2.0) as i32;
                    let text_y = ((size as f32 - estimated_letter_height) / 2.0) as i32;
                    
                    // Draw letter multiple times for BOLD effect
                    for offset_x in 0..2 {
                        for offset_y in 0..2 {
                            draw_text_mut(
                                &mut img, 
                                text_color, 
                                text_x + offset_x, 
                                text_y + offset_y, 
                                scale, 
                                &font, 
                                &letter_str
                            );
                        }
                    }
                }
                
                images.push(image::DynamicImage::ImageRgba8(img));
            }
            
            // Set all button images for this frame
            if let Some(ref mut device) = self.device {
                for (idx, img) in images.into_iter().enumerate() {
                    let _ = device.set_button_image(idx as u8, img);
                }
                let _ = device.flush();
            }
            
            // Frame delay (30ms per frame = ~33 FPS, smooth animation)
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        
        println!("[Stream Deck] ✅ Loading animation complete");
        Ok(())
    }
    
    /// Keep the gradient background animating (call from watcher until FX loaded)
    /// This handles BOTH initial reveal and infinite loop
    pub fn continue_loading_background(&mut self, frame: usize) -> Result<(), String> {
        if self.device.is_none() {
            return Err("No device".to_string());
        }
        
        if !self.loading_animation_active {
            // Log every 1000 frames (about every 6 seconds at 166 FPS)
            if frame % 1000 == 0 {
                let has_buttons = self.button_layout.iter().any(|b| b.is_some());
                crate::file_logger::log(&format!("[StreamDeck] ⏸️ Animation inactive (frame {}), has_buttons={}", 
                    frame, has_buttons));
            }
            return Err("Animation stopped".to_string());
        }
        
        // Calculate animation phase based on frame number
        let text_battles = "BATTLES";
        let text_loading = "LOADING";
        
        // Each letter appears every 1 frame at 166 FPS
        let frames_per_letter = 1;
        let battles_frames = text_battles.len() * frames_per_letter;
        let loading_frames = text_loading.len() * frames_per_letter;
        let hold_frames = 25; // Hold for ~150ms
        let cycle_frames = battles_frames + loading_frames + hold_frames;
        
        // Loop the animation by taking modulo
        let cycle_frame = frame % cycle_frames;
        
        let battles_visible = if cycle_frame < battles_frames {
            (cycle_frame / frames_per_letter).min(text_battles.len())
        } else {
            text_battles.len()
        };
        
        let loading_visible = if cycle_frame >= battles_frames {
            ((cycle_frame - battles_frames) / frames_per_letter).min(text_loading.len())
        } else {
            0
        };
        
        let size = self.get_button_size();
        let button_count = self.button_count();
        
        let (cols, _rows) = match self.device_kind {
            Some(Kind::Original) | Some(Kind::OriginalV2) | Some(Kind::Mk2) | Some(Kind::Mk2Scissor) => (5, 3),
            Some(Kind::Mini) | Some(Kind::MiniMk2) => (3, 2),
            Some(Kind::Xl) | Some(Kind::XlV2) => (8, 4),
            Some(Kind::Plus) | Some(Kind::Neo) => (4, 2),
            Some(Kind::Pedal) => (3, 1),
            None => return Err("Unknown device type".to_string()),
        };
        
        let logo_colors = [
            Rgba([238, 43, 99, 255]),   // Pink
            Rgba([255, 255, 255, 255]), // White
            Rgba([233, 179, 32, 255]),  // Yellow
        ];
        
        let font_data = include_bytes!("../assets/DejaVuSans.ttf");
        let font = FontRef::try_from_slice(font_data)
            .map_err(|e| format!("Failed to load font: {:?}", e))?;
        
        let mut images = Vec::new();
        
        for button_idx in 0..button_count {
            let row = button_idx / cols;
            let col = button_idx % cols;
            
            // LOGO BUTTON: First key of second row (row 1, col 0)
            let logo_button_idx = cols; // Row 1, Col 0 = 1 * cols + 0
            if button_idx == logo_button_idx {
                if let Ok(logo_img) = self.create_logo_button(frame) {
                    images.push(logo_img);
                    continue;
                }
            }
            
            let mut img = RgbaImage::new(size, size);
            
            // Continuous dark gradient animation
            let wave_offset = frame as f32 * 8.0;
            let position_factor = (col as f32 + row as f32) * 25.0;
            let hue = (position_factor + wave_offset) % 360.0;
            let (r, g, b) = Self::hsv_to_rgb(hue, 0.3, 0.2);
            
            draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), Rgba([r, g, b, 255]));
            
            // Keep text visible with progressive reveal (adjust for logo taking first button of row 1)
            let text_col = if row == 1 && col > 0 { col - 1 } else { col };
            let should_show_battles = row == 1 && col > 0 && text_col < text_battles.len() && text_col < battles_visible;
            let should_show_loading = row == 2 && col < text_loading.len() && col < loading_visible;
            
            if should_show_battles || should_show_loading {
                let (letter, color_idx) = if should_show_battles {
                    (text_battles.chars().nth(text_col).unwrap(), text_col % logo_colors.len())
                } else {
                    (text_loading.chars().nth(col).unwrap(), col % logo_colors.len())
                };
                
                let scale = PxScale::from((size as f32 * 0.65).max(50.0));
                let letter_str = letter.to_string();
                let text_color = logo_colors[color_idx];
                
                let estimated_letter_width = scale.x * 0.6;
                let estimated_letter_height = scale.y;
                let text_x = ((size as f32 - estimated_letter_width) / 2.0) as i32;
                let text_y = ((size as f32 - estimated_letter_height) / 2.0) as i32;
                
                // Bold text
                for offset_x in 0..2 {
                    for offset_y in 0..2 {
                        draw_text_mut(&mut img, text_color, text_x + offset_x, text_y + offset_y, scale, &font, &letter_str);
                    }
                }
            }
            
            images.push(image::DynamicImage::ImageRgba8(img));
        }
        
        if let Some(ref mut device) = self.device {
            for (idx, img) in images.into_iter().enumerate() {
                let _ = device.set_button_image(idx as u8, img);
            }
            let _ = device.flush();
        }
        
        Ok(())
    }
    
    /// Update button layout with FX buttons  
    /// Battle board effects go on left side, user FX on right side
    pub fn update_layout(&mut self, battle_board: Vec<FxButton>, user_fx: Vec<FxButton>) -> Result<(), String> {
        crate::file_logger::log(&format!("[StreamDeck] update_layout() START - Battle:{} UserFX:{}", 
            battle_board.len(), user_fx.len()));
        crate::file_logger::log(&format!("[StreamDeck]   Current animation_active={}, button_layout.len()={}", 
            self.loading_animation_active, self.button_layout.len()));
        
        // ONLY stop loading animation if we have actual button data
        // Don't stop for empty updates (0 battle board, 0 user FX) - keep animation playing
        if !battle_board.is_empty() || !user_fx.is_empty() {
            crate::file_logger::log("[StreamDeck]   Has button data - will process...");
            self.stop_loading_animation();
            crate::file_logger::log(&format!("[StreamDeck]   After stop_loading_animation: animation_active={}", 
                self.loading_animation_active));
            
            // Build image cache for fast lookups and trigger downloads (IMPORTANT!)
            crate::file_logger::log("[StreamDeck]   Building image cache...");
            self.image_cache.clear();
            let mut cached_count = 0;
            let mut download_count = 0;
            for fx in battle_board.iter().chain(user_fx.iter()) {
                // Use ID-prefixed cache key: "global_21_snipe" or "user_5_explosion"
                let cache_key = format!("{}_{}", fx.id, fx.name);
                if let Some(cached_path) = self.find_cached_image_internal(&cache_key) {
                    self.image_cache.insert(cache_key, cached_path);
                    cached_count += 1;
                }
                // Download if not cached (non-blocking)
                self.download_image_to_cache(fx);
                download_count += 1;
            }
            crate::file_logger::log(&format!("[StreamDeck]   Image cache: {}/{} cached, {} downloads triggered", 
                cached_count, download_count, download_count));
            
            // Update button layout mapping - SIMPLIFIED LAYOUT:
            // StreamDeck XL: 8 columns x 4 rows = 32 buttons
            // Battle Board: First 5 columns, top 3 rows = 15 buttons (positions: 0-4, 8-12, 16-20)
            // User FX: Last 3 columns, all 4 rows = 12 buttons (positions: 5-7, 13-15, 21-23, 29-31)
            // Empty: Bottom left 5 buttons (positions: 24-28)
            crate::file_logger::log("[StreamDeck]   Clearing and resizing button_layout...");
            self.button_layout.clear();
            self.button_layout.resize(self.button_count(), None);
            crate::file_logger::log(&format!("[StreamDeck]   button_layout resized to {}", self.button_count()));
            
            // Map FX buttons to Stream Deck positions
            crate::file_logger::log("[StreamDeck]   Mapping buttons with new layout...");
            let mut mapped_count = 0;
            
            // 1. Battle board: First 5 columns, top 3 rows (15 buttons max)
            let battle_positions = [
                // Row 0 (cols 0-4)
                0, 1, 2, 3, 4,
                // Row 1 (cols 0-4)
                8, 9, 10, 11, 12,
                // Row 2 (cols 0-4)
                16, 17, 18, 19, 20,
            ];
            
            for (i, &pos) in battle_positions.iter().enumerate() {
                if i < battle_board.len() {
                    self.button_layout[pos] = Some(ButtonType::FxButton(battle_board[i].clone()));
                    mapped_count += 1;
                }
            }
            
            // 2. User FX: Last 3 columns, all 4 rows (12 buttons max)
            let fx_positions = [
                // Row 0 (cols 5-7)
                5, 6, 7,
                // Row 1 (cols 5-7)
                13, 14, 15,
                // Row 2 (cols 5-7)
                21, 22, 23,
                // Row 3 (cols 5-7)
                29, 30, 31,
            ];
            
            for (i, &pos) in fx_positions.iter().enumerate() {
                if i < user_fx.len() {
                    self.button_layout[pos] = Some(ButtonType::FxButton(user_fx[i].clone()));
                    mapped_count += 1;
                }
            }
            
            // Positions 24-28 remain empty (bottom left 5 buttons)
            
            crate::file_logger::log(&format!("[StreamDeck]   Mapped {} buttons to layout (Battle:{}, UserFX:{})", 
                mapped_count, battle_board.len().min(15), user_fx.len().min(12)));
            
            // Render buttons to the device
            crate::file_logger::log("[StreamDeck]   Calling render_all_buttons()...");
            self.render_all_buttons()?;
            crate::file_logger::log("[StreamDeck]   render_all_buttons() completed successfully");
        } else {
            crate::file_logger::log("[StreamDeck]   Empty update - animation continues");
        }
        
        crate::file_logger::log(&format!("[StreamDeck] update_layout() END - animation_active={}", 
            self.loading_animation_active));
        Ok(())
    }
    
    /// Find cached image from frontend cache (NO downloading - images are pre-cached by frontend!)
    /// Cache files are named with ID prefix, e.g., "global_21_snipe.jpg", "user_5_explosion.png"
    fn find_cached_image(&self, fx_button: &FxButton) -> Option<PathBuf> {
        // Construct the cache key with ID prefix
        let cache_key = format!("{}_{}", fx_button.id, fx_button.name);
        
        crate::file_logger::log(&format!("[StreamDeck] 🔍 Finding image for: {} (cache_key: {})", 
            fx_button.name, cache_key));
        
        // Check in-memory cache first (fast)
        if let Some(path) = self.image_cache.get(&cache_key) {
            crate::file_logger::log(&format!("[StreamDeck]   ✅ Found in memory cache: {}", path.display()));
            return Some(path.clone());
        }
        
        // Fallback to filesystem search (slow)
        crate::file_logger::log("[StreamDeck]   🔎 Not in memory, searching filesystem...");
        let result = self.find_cached_image_internal(&cache_key);
        
        if let Some(ref path) = result {
            crate::file_logger::log(&format!("[StreamDeck]   ✅ Found on filesystem: {}", path.display()));
        } else {
            crate::file_logger::log(&format!("[StreamDeck]   ❌ Not found (will show gradient/icon)"));
        }
        
        result
    }
    
    /// Get the cache directory (persistent across app restarts)
    /// Uses AppData\Local\BattlesDesktop\cache for proper Windows compatibility
    fn get_cache_dir() -> PathBuf {
        // Use Windows AppData\Local for production (writable by user, persistent)
        if let Some(local_data) = dirs::data_local_dir() {
            let cache_dir = local_data.join("BattlesDesktop").join("streamdeck_cache");
            crate::file_logger::log(&format!("[StreamDeck Cache] Using AppData cache: {}", cache_dir.display()));
            return cache_dir;
        }
        
        // Fallback: Try executable directory (works in dev, might fail in production if installed to Program Files)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                let cache_dir = exe_dir.join("streamdeck_cache");
                crate::file_logger::log(&format!("[StreamDeck Cache] Fallback to exe dir cache: {}", cache_dir.display()));
                return cache_dir;
            }
        }
        
        // Final fallback to temp dir
        let cache_dir = std::env::temp_dir().join("battles_fx_cache");
        crate::file_logger::log(&format!("[StreamDeck Cache] Fallback to temp cache: {}", cache_dir.display()));
        cache_dir
    }
    
    /// Internal implementation that actually searches the filesystem
    fn find_cached_image_internal(&self, fx_name: &str) -> Option<PathBuf> {
        let cache_dir = Self::get_cache_dir();
        
        crate::file_logger::log(&format!("[StreamDeck]   Cache dir: {}", cache_dir.display()));
        
        if !cache_dir.exists() {
            crate::file_logger::log(&format!("[StreamDeck]   ⚠️ Cache directory does not exist"));
            return None;
        }
        
        crate::file_logger::log(&format!("[StreamDeck]   ✅ Cache directory exists, searching..."));
        
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
                            if ext_str == "jpg" || ext_str == "jpeg" || ext_str == "png" || ext_str == "webp" || ext_str == "gif" || ext_str == "avif" {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }
        
        // Also try direct patterns with the FX name
        let possible_patterns = vec![
            format!("{}.webp", fx_name), // Check WebP first (preferred)
            format!("{}.jpg", fx_name),
            format!("{}.jpeg", fx_name),
            format!("{}.png", fx_name),
            format!("{}.avif", fx_name),
            format!("{}.gif", fx_name),
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
        crate::file_logger::log(&format!("[StreamDeck] 📥 Starting download for: {} (id: {})", 
            fx_button.name, fx_button.id));
        
        if fx_button.image_url.is_none() {
            crate::file_logger::log(&format!("[StreamDeck]   ⚠️ No image URL for FX: {}", fx_button.name));
            return;
        }
        
        let cache_dir = Self::get_cache_dir();
        crate::file_logger::log(&format!("[StreamDeck]   Cache directory: {}", cache_dir.display()));
        
        if let Err(e) = std::fs::create_dir_all(&cache_dir) {
            crate::file_logger::log(&format!("[StreamDeck]   ❌ Failed to create cache directory: {}", e));
            return;
        }
        
        crate::file_logger::log(&format!("[StreamDeck]   ✅ Cache directory ready"));
        
        // Cache filename: Try to preserve extension from URL or default to .jpg
        let extension = if let Some(url) = &fx_button.image_url {
            if url.contains(".webp") {
                "webp"
            } else if url.contains(".png") {
                "png"
            } else if url.contains(".avif") {
                "avif"
            } else {
                "jpg"
            }
        } else {
            "jpg"
        };
        // IMPORTANT: Use ID in cache filename to prevent collisions between global and user FX with same name!
        // e.g., "global_21_snipe.jpg" vs "user_5_snipe.jpg"
        let cache_filename = format!("{}_{}.{}", fx_button.id, fx_button.name, extension);
        let cache_path = cache_dir.join(&cache_filename);
        
        // Delete old cached files with different extensions FIRST
        // Also delete OLD FORMAT files (without ID prefix) for migration
        let all_extensions = vec!["webp", "jpg", "jpeg", "png", "avif", "gif"];
        for old_ext in all_extensions {
            if old_ext != extension {
                // Delete new format with different extension: global_21_snipe.png when global_21_snipe.jpg is requested
                let old_cache_path = cache_dir.join(format!("{}_{}.{}", fx_button.id, fx_button.name, old_ext));
                if old_cache_path.exists() {
                    let _ = std::fs::remove_file(&old_cache_path);
                }
            }
            // ALSO delete old format WITHOUT ID prefix for migration: snipe.jpg -> global_21_snipe.jpg
            let legacy_cache_path = cache_dir.join(format!("{}.{}", fx_button.name, old_ext));
            if legacy_cache_path.exists() {
                let _ = std::fs::remove_file(&legacy_cache_path);
            }
        }
        
        // Skip if already cached (check AFTER cleanup)
        if cache_path.exists() {
            crate::file_logger::log(&format!("[StreamDeck]   ✅ Already cached: {}", cache_path.display()));
            return;
        }
        
        crate::file_logger::log(&format!("[StreamDeck]   🌐 Starting HTTP download..."));
        
        // Download from Nuxt proxy (non-blocking in background)
        let image_url = fx_button.image_url.clone().unwrap();
        let name = fx_button.name.clone();
        let fx_id = fx_button.id.clone();
        let cache_path_clone = cache_path.clone();
        
        std::thread::spawn(move || {
            // Use different base URL based on build mode
            #[cfg(debug_assertions)]
            let base_url = "https://battles.app";
            #[cfg(not(debug_assertions))]
            let base_url = "https://battles.app";
            
            let full_url = format!("{}{}", base_url, image_url);
            
            crate::file_logger::log(&format!("[StreamDeck Download] GET {}", full_url));
            
            match reqwest::blocking::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(10))
                .build()
            {
                Ok(client) => {
                    match client.get(&full_url).send() {
                        Ok(response) if response.status().is_success() => {
                            crate::file_logger::log(&format!("[StreamDeck Download] ✅ HTTP 200 for {}", name));
                            match response.bytes() {
                                Ok(bytes) => {
                                    crate::file_logger::log(&format!("[StreamDeck Download] 📦 Downloaded {} bytes for {}", 
                                        bytes.len(), name));
                                    
                                    if let Err(e) = std::fs::write(&cache_path_clone, &bytes) {
                                        crate::file_logger::log(&format!("[StreamDeck Download] ❌ Failed to write cache file: {}", e));
                                    } else {
                                        crate::file_logger::log(&format!("[StreamDeck Download] ✅ Cached to: {}", 
                                            cache_path_clone.display()));
                                        
                                        // Trigger re-render of this specific button
                                        let mut manager_lock = STREAMDECK_MANAGER.lock();
                                        if let Some(ref mut manager) = *manager_lock {
                                            // Find and re-render the button
                                            let _ = manager.refresh_button_by_id(&fx_id);
                                            crate::file_logger::log(&format!("[StreamDeck Download] 🔄 Triggered button refresh for {}", name));
                                        }
                                    }
                                }
                                Err(e) => crate::file_logger::log(&format!("[StreamDeck Download] ❌ Failed to read response bytes: {}", e)),
                            }
                        }
                        Ok(response) => crate::file_logger::log(&format!("[StreamDeck Download] ⚠️ HTTP {} for {}", 
                            response.status(), name)),
                        Err(e) => crate::file_logger::log(&format!("[StreamDeck Download] ❌ Request failed for {}: {}", name, e)),
                    }
                }
                Err(e) => crate::file_logger::log(&format!("[StreamDeck Download] ❌ Failed to create HTTP client: {}", e)),
            }
        });
    }
    
    fn update_layout_internal(&mut self, battle_board: Vec<FxButton>, user_fx: Vec<FxButton>) -> Result<(), String> {
        let button_count = self.button_count();
        if button_count == 0 {
            println!("[Stream Deck] ❌ ERROR: No device connected");
            return Err("No device connected".to_string());
        }
        
        
        // Build image cache for fast lookups (avoid repeated filesystem searches)
        self.image_cache.clear();
        for fx in battle_board.iter().chain(user_fx.iter()) {
            // Use ID-prefixed cache key: "global_21_snipe" or "user_5_explosion"
            let cache_key = format!("{}_{}", fx.id, fx.name);
            if let Some(cached_path) = self.find_cached_image_internal(&cache_key) {
                self.image_cache.insert(cache_key, cached_path);
            }
            // Download if not cached (non-blocking)
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
                            self.button_layout[button_idx] = Some(ButtonType::FxButton(battle_board[battle_index].clone()));
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
                            self.button_layout[button_idx] = Some(ButtonType::FxButton(user_fx[user_index].clone()));
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
                    self.button_layout[button_idx] = Some(ButtonType::FxButton(FxButton {
                        id: format!("control_{}", name.to_lowercase()),
                        name: name.to_string(),
                        image_url: None,
                        is_global: false,
                        position: row,
                    }));
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
                            self.button_layout[button_idx] = Some(ButtonType::FxButton(battle_board[battle_index].clone()));
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
                            self.button_layout[button_idx] = Some(ButtonType::FxButton(user_fx[user_index].clone()));
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
        crate::file_logger::log("[StreamDeck]     render_all_buttons() START");
        
        if self.device.is_none() {
            crate::file_logger::log("[StreamDeck]     ⚠️ No device connected, skipping render");
            return Ok(());
        }
        
        crate::file_logger::log(&format!("[StreamDeck]     Rendering {} buttons...", self.button_layout.len()));
        
        // Collect all button images first
        let mut button_images: Vec<(u8, Option<image::DynamicImage>)> = Vec::new();
        let mut with_image_count = 0;
        let mut without_image_count = 0;
        
        for (idx, button_type_opt) in self.button_layout.iter().enumerate() {
            let image = if let Some(button_type) = button_type_opt {
                match button_type {
                    ButtonType::FxButton(fx_button) => {
                        let is_playing = self.button_states
                            .get(&(idx as u8))
                            .map(|s| s.is_playing)
                            .unwrap_or(false);
                        
                        let has_cached_image = self.find_cached_image(fx_button).is_some();
                        if has_cached_image {
                            with_image_count += 1;
                        } else {
                            without_image_count += 1;
                        }
                        
                        Some(self.create_button_image(fx_button, is_playing)?)
                    }
                    ButtonType::TvMonitorControl => {
                        Some(self.create_tv_monitor_button()?)
                    }
                    ButtonType::VideoToggle => {
                        let is_playing = self.button_states
                            .get(&(idx as u8))
                            .map(|s| s.is_playing)
                            .unwrap_or(false);
                        Some(self.create_video_toggle_button(is_playing)?)
                    }
                    ButtonType::VideoLoopUp => {
                        Some(self.create_video_loop_button(true)?)
                    }
                    ButtonType::VideoLoopDown => {
                        Some(self.create_video_loop_button(false)?)
                    }
                }
            } else {
                // Empty button - apply gradient based on position
                let _size = self.get_button_size();
                let img = self.create_empty_button_with_gradient(idx);
                Some(image::DynamicImage::ImageRgba8(img))
            };
            button_images.push((idx as u8, image));
        }
        
        crate::file_logger::log(&format!("[StreamDeck]     Images: {} with cached, {} without (text only)", 
            with_image_count, without_image_count));
        
        // Now set all button images
        if let Some(ref mut device) = self.device {
            crate::file_logger::log(&format!("[StreamDeck]     Setting {} button images to device...", button_images.len()));
            for (idx, image_opt) in button_images {
                if let Some(image) = image_opt {
                    device.set_button_image(idx, image)
                        .map_err(|e| format!("Failed to set button image: {}", e))?;
                }
            }
            
            // Flush changes to device
            crate::file_logger::log("[StreamDeck]     Flushing to device...");
            device.flush().map_err(|e| format!("Failed to flush device: {}", e))?;
            crate::file_logger::log("[StreamDeck]     ✅ Flush completed");
        }
        
        crate::file_logger::log("[StreamDeck]     render_all_buttons() END");
        Ok(())
    }
    
    /// Detect media type from filename
    fn detect_media_type(filename: &str) -> Option<&'static str> {
        let filename_lower = filename.to_lowercase();
        if filename_lower.contains(".mp4") || filename_lower.contains(".webm") || filename_lower.contains(".mov") || filename_lower.contains(".avi") {
            Some("video")
        } else if filename_lower.contains(".mp3") || filename_lower.contains(".wav") || filename_lower.contains(".ogg") || filename_lower.contains(".m4a") {
            Some("audio")
        } else {
            None
        }
    }
    
    /// Draw a simple video icon (play triangle in a rectangle)
    fn draw_video_icon(img: &mut RgbaImage, center_x: i32, center_y: i32, icon_size: i32) {
        let white = image::Rgba([255, 255, 255, 200]);
        
        // Draw rectangle frame
        let rect_width = icon_size;
        let rect_height = (icon_size as f32 * 0.7) as i32;
        let rect = Rect::at(center_x - rect_width / 2, center_y - rect_height / 2).of_size(rect_width as u32, rect_height as u32);
        imageproc::drawing::draw_hollow_rect_mut(img, rect, white);
        
        // Draw play triangle inside
        let triangle_size = icon_size / 3;
        for dy in -triangle_size..triangle_size {
            let width = (triangle_size as f32 * (1.0 - dy.abs() as f32 / triangle_size as f32)) as i32;
            for dx in 0..width {
                let x = (center_x + dx) as u32;
                let y = (center_y + dy) as u32;
                if x < img.width() && y < img.height() {
                    img.put_pixel(x, y, white);
                }
            }
        }
    }
    
    /// Draw a simple audio icon (musical note)
    fn draw_audio_icon(img: &mut RgbaImage, center_x: i32, center_y: i32, icon_size: i32) {
        let white = image::Rgba([255, 255, 255, 200]);
        
        // Draw note stem
        let stem_height = icon_size;
        let stem_x = center_x + icon_size / 4;
        for y in (center_y - stem_height / 2)..(center_y + stem_height / 4) {
            if y >= 0 && y < img.height() as i32 {
                for dx in -1..=1 {
                    let x = stem_x + dx;
                    if x >= 0 && x < img.width() as i32 {
                        img.put_pixel(x as u32, y as u32, white);
                    }
                }
            }
        }
        
        // Draw note head (circle)
        let note_radius = icon_size / 4;
        draw_filled_circle_mut(img, (stem_x, center_y + stem_height / 4), note_radius as i32, white);
    }
    
    /// Determine button section and position for gradient
    fn get_button_section(&self, idx: usize) -> (bool, usize) {
        // For XL/XLV2: 8 columns x 4 rows = 32 buttons
        // Battle Board: cols 0-4 (rows 0-2) = positions 0-4, 8-12, 16-20
        // FX Board: cols 5-7 (rows 0-3) = positions 5-7, 13-15, 21-23, 29-31
        // Empty: row 3, cols 0-4 = positions 24-28
        
        let row = idx / 8;
        let col = idx % 8;
        
        if col < 5 {
            // Battle board section (left side)
            let position = row * 5 + col;
            (true, position)
        } else {
            // FX board section (right side)
            let position = row * 3 + (col - 5);
            (false, position)
        }
    }
    
    /// Create an empty button with gradient background based on position
    fn create_empty_button_with_gradient(&self, idx: usize) -> RgbaImage {
        let size = self.get_button_size();
        let (is_battle_board, position) = self.get_button_section(idx);
        
        Self::create_gradient_background(size, is_battle_board, position, false)
    }
    
    /// Create a gradient background for a button section
    fn create_gradient_background(size: u32, is_battle_board: bool, button_position: usize, is_playing: bool) -> RgbaImage {
        let mut img = RgbaImage::new(size, size);
        
        if is_battle_board {
            // Battle Board: Purple to Pink gradient (matching dashboard: from-purple-900/30 to-pink-900/30)
            // Purple-900 is approximately rgb(88, 28, 135), Pink-900 is approximately rgb(131, 24, 67)
            let purple_start = [88, 28, 135];
            let pink_end = [131, 24, 67];
            
            // Calculate gradient position based on button position (0-19 for battle board)
            // Positions: 0-4 (row 0), 5-9 (row 1), 10-14 (row 2), 15-19 (row 3)
            let gradient_progress = (button_position as f32 / 19.0).min(1.0);
            
            for y in 0..size {
                for x in 0..size {
                    // Combined gradient: left-to-right based on button position + top-to-bottom within button
                    let horizontal_progress = gradient_progress;
                    let vertical_progress = y as f32 / size as f32;
                    let combined_progress = (horizontal_progress * 0.7 + vertical_progress * 0.3).min(1.0);
                    
                    let r = (purple_start[0] as f32 + (pink_end[0] as f32 - purple_start[0] as f32) * combined_progress) as u8;
                    let g = (purple_start[1] as f32 + (pink_end[1] as f32 - purple_start[1] as f32) * combined_progress) as u8;
                    let b = (purple_start[2] as f32 + (pink_end[2] as f32 - purple_start[2] as f32) * combined_progress) as u8;
                    
                    // Apply opacity and brightness based on playing state
                    let (r, g, b) = if is_playing {
                        // When playing: use full brightness (100% opacity) and boost by 1.8x
                        let r = ((r as f32 * 1.8).min(255.0) as u8).max(40);
                        let g = ((g as f32 * 1.8).min(255.0) as u8).max(40);
                        let b = ((b as f32 * 1.8).min(255.0) as u8).max(40);
                        (r, g, b)
                    } else {
                        // When not playing: apply 30% opacity by blending with black
                        let r = ((r as f32 * 0.3) as u8).max(10);
                        let g = ((g as f32 * 0.3) as u8).max(10);
                        let b = ((b as f32 * 0.3) as u8).max(10);
                        (r, g, b)
                    };
                    
                    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                }
            }
        } else {
            // FX Board: Gray background (matching dashboard: bg-gray-800/50)
            // Gray-800 is approximately rgb(31, 41, 55)
            let gray = [31, 41, 55];
            
            for y in 0..size {
                for x in 0..size {
                    let (r, g, b) = if is_playing {
                        // When playing: use brighter gray (boost by 3x)
                        let r = ((gray[0] as f32 * 3.0).min(255.0) as u8).max(90);
                        let g = ((gray[1] as f32 * 3.0).min(255.0) as u8).max(120);
                        let b = ((gray[2] as f32 * 3.0).min(255.0) as u8).max(160);
                        (r, g, b)
                    } else {
                        // When not playing: apply 50% opacity by blending with black
                        let r = ((gray[0] as f32 * 0.5) as u8).max(15);
                        let g = ((gray[1] as f32 * 0.5) as u8).max(15);
                        let b = ((gray[2] as f32 * 0.5) as u8).max(15);
                        (r, g, b)
                    };
                    
                    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                }
            }
        }
        
        img
    }
    
    /// Create button image with text and styling
    fn create_button_image(&self, fx_button: &FxButton, is_playing: bool) -> Result<image::DynamicImage, String> {
        // Get button size
        let size = self.get_button_size();
        
        crate::file_logger::log(&format!("[StreamDeck] 🎨 Creating button image for: {} (playing: {})", 
            fx_button.name, is_playing));
        
        // Try to load cached image from frontend cache (NO downloading!)
        // Cache files are named with ID prefix, e.g., "global_21_snipe.jpg", "user_5_explosion.png"
        let cached_image = if let Some(cached_path) = self.find_cached_image(fx_button) {
            crate::file_logger::log(&format!("[StreamDeck]   📂 Loading image from: {}", cached_path.display()));
            match image::open(&cached_path) {
                Ok(img) => {
                    crate::file_logger::log(&format!("[StreamDeck]   ✅ Image loaded successfully"));
                    Some(img)
                }
                Err(e) => {
                    crate::file_logger::log(&format!("[StreamDeck]   ❌ Failed to load image: {}", e));
                    None
                }
            }
        } else {
            crate::file_logger::log(&format!("[StreamDeck]   📭 No cached image found"));
            None
        };
        
        // Detect media type from URL if no image
        let media_type = if cached_image.is_none() {
            fx_button.image_url.as_ref().and_then(|url| Self::detect_media_type(url))
        } else {
            None
        };
        
        // Create base button image
        let mut img = if let Some(cached_img) = cached_image {
            // Create gradient background based on button type and playing state
            let mut gradient_bg = Self::create_gradient_background(size, fx_button.is_global, fx_button.position, is_playing);
            
            // Resize the cached image
            let resized = cached_img.resize_exact(size, size, image::imageops::FilterType::Triangle);
            let rgba_img = resized.to_rgba8();
            
            // Composite the image onto the gradient background (preserves PNG transparency)
            for y in 0..size {
                for x in 0..size {
                    let pixel = rgba_img.get_pixel(x, y);
                    let alpha = pixel[3] as f32 / 255.0;
                    
                    if alpha > 0.0 {
                        let bg_pixel = gradient_bg.get_pixel(x, y);
                        let r = ((pixel[0] as f32 * alpha) + (bg_pixel[0] as f32 * (1.0 - alpha))) as u8;
                        let g = ((pixel[1] as f32 * alpha) + (bg_pixel[1] as f32 * (1.0 - alpha))) as u8;
                        let b = ((pixel[2] as f32 * alpha) + (bg_pixel[2] as f32 * (1.0 - alpha))) as u8;
                        
                        gradient_bg.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                    }
                }
            }
            
            gradient_bg
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
            } else {
                // Use gradient background for normal FX buttons without images (brightness varies by playing state)
                img = Self::create_gradient_background(size, fx_button.is_global, fx_button.position, is_playing);
                image::Rgba([0, 0, 0, 0]) // Transparent - we already have the gradient
            };
            
            // Only draw solid color if not using gradient (control buttons only)
            if bg_color[3] > 0 {
                draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), bg_color);
            }
            
            // Draw media icon if video/audio and no custom image
            if let Some(media) = media_type {
                let icon_size = (size as f32 * 0.4) as i32;
                let center_x = (size / 2) as i32;
                let center_y = (size as f32 * 0.4) as i32;
                
                match media {
                    "video" => Self::draw_video_icon(&mut img, center_x, center_y, icon_size),
                    "audio" => Self::draw_audio_icon(&mut img, center_x, center_y, icon_size),
                    _ => {}
                }
            }
            
            img
        };
        
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
    
    /// Create TV Monitor Control button (purple with TV icon)
    fn create_tv_monitor_button(&self) -> Result<image::DynamicImage, String> {
        let size = self.get_button_size();
        let mut img = RgbaImage::new(size, size);
        
        // Purple background
        let purple = image::Rgba([138, 43, 226, 255]);
        draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), purple);
        
        // Draw simple TV icon (rectangle with stand)
        let white = image::Rgba([255, 255, 255, 255]);
        let icon_size = (size as f32 * 0.5) as i32;
        let center_x = (size / 2) as i32;
        let center_y = (size as f32 * 0.4) as i32;
        
        // TV screen
        let screen = Rect::at(center_x - icon_size / 2, center_y - icon_size / 3)
            .of_size(icon_size as u32, (icon_size as f32 * 0.6) as u32);
        imageproc::drawing::draw_hollow_rect_mut(&mut img, screen, white);
        for i in 0..2 {
            let inner_screen = Rect::at(
                center_x - icon_size / 2 + i,
                center_y - icon_size / 3 + i
            ).of_size((icon_size - i * 2) as u32, ((icon_size as f32 * 0.6) - i as f32 * 2.0) as u32);
            imageproc::drawing::draw_hollow_rect_mut(&mut img, inner_screen, white);
        }
        
        // TV stand
        let stand_width = icon_size / 4;
        let stand_height = icon_size / 6;
        let stand_rect = Rect::at(center_x - stand_width / 2, center_y + icon_size / 4)
            .of_size(stand_width as u32, stand_height as u32);
        draw_filled_rect_mut(&mut img, stand_rect, white);
        
        // Add text
        let font_data = include_bytes!("../assets/DejaVuSans.ttf");
        let font = FontRef::try_from_slice(font_data)
            .map_err(|e| format!("Failed to load font: {:?}", e))?;
        let font_scale = PxScale::from((size as f32 * 0.12).max(10.0));
        let text = "TV";
        let text_x = ((size as f32 - (text.len() as f32 * font_scale.x * 0.5)) / 2.0) as i32;
        let text_y = (size as f32 * 0.75) as i32;
        draw_text_mut(&mut img, white, text_x, text_y, font_scale, &font, text);
        
        Ok(image::DynamicImage::ImageRgba8(img))
    }
    
    /// Create Video Toggle button (play/stop icon)
    fn create_video_toggle_button(&self, is_playing: bool) -> Result<image::DynamicImage, String> {
        let size = self.get_button_size();
        let mut img = RgbaImage::new(size, size);
        
        // Background color changes based on state
        let bg_color = if is_playing {
            image::Rgba([50, 205, 50, 255]) // Green when playing
        } else {
            image::Rgba([60, 60, 60, 255]) // Gray when stopped
        };
        draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), bg_color);
        
        let white = image::Rgba([255, 255, 255, 255]);
        let icon_size = (size as f32 * 0.4) as i32;
        let center_x = (size / 2) as i32;
        let center_y = (size as f32 * 0.4) as i32;
        
        if is_playing {
            // Draw stop icon (square)
            let stop_rect = Rect::at(center_x - icon_size / 2, center_y - icon_size / 2)
                .of_size(icon_size as u32, icon_size as u32);
            draw_filled_rect_mut(&mut img, stop_rect, white);
        } else {
            // Draw play icon (triangle)
            for dy in -icon_size..icon_size {
                let width = (icon_size as f32 * (1.0 - dy.abs() as f32 / icon_size as f32)) as i32;
                for dx in 0..width {
                    let x = (center_x + dx) as u32;
                    let y = (center_y + dy) as u32;
                    if x < img.width() && y < img.height() {
                        img.put_pixel(x, y, white);
                    }
                }
            }
        }
        
        // Add text
        let font_data = include_bytes!("../assets/DejaVuSans.ttf");
        let font = FontRef::try_from_slice(font_data)
            .map_err(|e| format!("Failed to load font: {:?}", e))?;
        let font_scale = PxScale::from((size as f32 * 0.10).max(10.0));
        let text = if is_playing { "STOP" } else { "PLAY" };
        let text_x = ((size as f32 - (text.len() as f32 * font_scale.x * 0.5)) / 2.0) as i32;
        let text_y = (size as f32 * 0.75) as i32;
        draw_text_mut(&mut img, white, text_x, text_y, font_scale, &font, text);
        
        Ok(image::DynamicImage::ImageRgba8(img))
    }
    
    /// Create Video Loop browser button (yellow with up/down arrow)
    fn create_video_loop_button(&self, is_up: bool) -> Result<image::DynamicImage, String> {
        let size = self.get_button_size();
        let mut img = RgbaImage::new(size, size);
        
        // Yellow background
        let yellow = image::Rgba([255, 215, 0, 255]);
        draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(size, size), yellow);
        
        let black = image::Rgba([0, 0, 0, 255]);
        let arrow_size = (size as f32 * 0.5) as i32;
        let center_x = (size / 2) as i32;
        let center_y = (size / 2) as i32;
        
        // Draw arrow (triangle pointing up or down)
        if is_up {
            // Up arrow
            for dy in 0..arrow_size {
                let width = (dy as f32 * 2.0) as i32;
                for dx in -width/2..width/2 {
                    let x = (center_x + dx) as u32;
                    let y = (center_y - arrow_size / 2 + dy) as u32;
                    if x < img.width() && y < img.height() {
                        img.put_pixel(x, y, black);
                    }
                }
            }
        } else {
            // Down arrow
            for dy in 0..arrow_size {
                let width = ((arrow_size - dy) as f32 * 2.0) as i32;
                for dx in -width/2..width/2 {
                    let x = (center_x + dx) as u32;
                    let y = (center_y - arrow_size / 2 + dy) as u32;
                    if x < img.width() && y < img.height() {
                        img.put_pixel(x, y, black);
                    }
                }
            }
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
    
    /// Read button presses (BLOCKING read - waits for real hardware events)
    /// This should be called from a dedicated blocking thread, NOT in async context
    pub fn read_button_presses(&mut self) -> Vec<u8> {
        let mut pressed_buttons = Vec::new();
        
        if let Some(ref mut device) = self.device {
            // INSTANT non-blocking read (0ms timeout = immediate return)
            // Returns immediately if no button press, allowing continuous polling without any delay
            match device.read_input(Some(std::time::Duration::from_millis(0))) {
                Ok(input) => {
                    match input {
                        elgato_streamdeck::StreamDeckInput::ButtonStateChange(states) => {
                            // Stream Deck returns current state of all buttons
                            // DEBOUNCING: Only emit events on RISING EDGE (button press, not hold)
                            for (idx, is_pressed) in states.iter().enumerate() {
                                let button_idx = idx as u8;
                                let was_pressed = self.last_button_states.get(&button_idx).copied().unwrap_or(false);
                                
                                // Only trigger event if button was just pressed (transition from false to true)
                                if *is_pressed && !was_pressed {
                                    pressed_buttons.push(button_idx);
                                    println!("[SD] BTN{} ▲", button_idx); // INSTANT console log (no file I/O)
                                }
                                
                                // Update last state
                                self.last_button_states.insert(button_idx, *is_pressed);
                            }
                            
                            // Button events detected (silent for performance)
                        }
                        _ => {
                            // Ignore other input types (encoders, touchscreen, etc.)
                        }
                    }
                }
                Err(_) => {
                    // Timeout (1 second passed with no button press) - this is normal
                }
            }
        }
        
        pressed_buttons
    }
    
    /// Handle button press (toggle play/stop)
    pub fn handle_button_press(&mut self, button_idx: u8) -> Option<ButtonPressEvent> {
        // DEDUPLICATION: Prevent duplicate events within 200ms
        let now = std::time::Instant::now();
        if let Some(last_time) = self.last_event_time.get(&button_idx) {
            let elapsed = now.duration_since(*last_time);
            if elapsed < std::time::Duration::from_millis(200) {
                // Silent debounce for performance
                return None; // Ignore duplicate event
            }
        }
        
        // Update last event time
        self.last_event_time.insert(button_idx, now);
        
        // Get the button type at this position
        let button_type = self.button_layout.get(button_idx as usize)?.as_ref()?.clone();
        
        match button_type {
            ButtonType::FxButton(fx_button) => {
                // Toggle playing state for FX buttons
                let entry = self.button_states
                    .entry(button_idx)
                    .or_insert(ButtonState {
                        is_playing: false,
                        button: Some(fx_button.clone()),
                    });
                
                entry.is_playing = !entry.is_playing;
                let new_state = entry.is_playing;
                let fx_id = fx_button.id.clone();
                
                // DON'T update visual here - let frontend control it via set_button_state
                // This prevents double-rendering (once here, once when frontend calls set_button_state)
                
                Some(ButtonPressEvent::FxPressed { fx_id, is_playing: new_state })
            }
            ButtonType::TvMonitorControl => {
                Some(ButtonPressEvent::TvMonitorToggle)
            }
            ButtonType::VideoToggle => {
                // Toggle video playing state
                let entry = self.button_states
                    .entry(button_idx)
                    .or_insert(ButtonState {
                        is_playing: false,
                        button: None,
                    });
                
                entry.is_playing = !entry.is_playing;
                let new_state = entry.is_playing;
                
                // Update button visual
                if self.device.is_some() {
                    if let Ok(image) = self.create_video_toggle_button(new_state) {
                        if let Some(ref mut device) = self.device {
                            let _ = device.set_button_image(button_idx, image);
                            let _ = device.flush();
                        }
                    }
                }
                
                Some(ButtonPressEvent::VideoToggle { is_playing: new_state })
            }
            ButtonType::VideoLoopUp => {
                // Increment video loop index (wrap around)
                self.current_video_loop_index = self.current_video_loop_index.wrapping_add(1);
                Some(ButtonPressEvent::VideoLoopBrowse { 
                    direction: 1, 
                    loop_index: self.current_video_loop_index 
                })
            }
            ButtonType::VideoLoopDown => {
                // Decrement video loop index (wrap around)
                self.current_video_loop_index = self.current_video_loop_index.wrapping_sub(1);
                Some(ButtonPressEvent::VideoLoopBrowse { 
                    direction: -1, 
                    loop_index: self.current_video_loop_index 
                })
            }
        }
    }
    
    /// Refresh a single button by FX ID (called after image downloads)
    pub fn refresh_button_by_id(&mut self, fx_id: &str) -> Result<(), String> {
        // Find button with this FX ID
        let mut button_to_update: Option<(u8, FxButton)> = None;
        
        for (idx, button_type_opt) in self.button_layout.iter().enumerate() {
            if let Some(ButtonType::FxButton(fx_button)) = button_type_opt {
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
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Update button state WITHOUT rendering (called when FX stops playing)
    /// Use render_all_buttons() after batch updates to update visuals
    pub fn set_button_state(&mut self, fx_id: &str, is_playing: bool) -> Result<(), String> {
        // Find button with this FX ID and update state
        let mut button_to_update: Option<(u8, FxButton)> = None;
        
        for (idx, button_type_opt) in self.button_layout.iter().enumerate() {
            if let Some(ButtonType::FxButton(fx_button)) = button_type_opt {
                if fx_button.id == fx_id {
                    button_to_update = Some((idx as u8, fx_button.clone()));
                    break;
                }
            }
        }
        
        if let Some((idx, fx_button)) = button_to_update {
            // Update state ONLY - don't render yet
            self.button_states
                .entry(idx)
                .and_modify(|s| s.is_playing = is_playing)
                .or_insert(ButtonState {
                    is_playing,
                    button: Some(fx_button.clone()),
                });
            
            // Render button immediately (optimized - no flush)
            if self.device.is_some() {
                if let Ok(image) = self.create_button_image(&fx_button, is_playing) {
                    if let Some(ref mut device) = self.device {
                        // Set image but DON'T flush - caller will flush after batch
                        let _ = device.set_button_image(idx, image);
                    }
                }
            }
            
            Ok(())
        } else {
            Err(format!("FX ID '{}' not found in button layout", fx_id))
        }
    }
    
    /// Flush all pending button updates to the device
    pub fn flush_updates(&mut self) -> Result<(), String> {
        if let Some(ref mut device) = self.device {
            device.flush().map_err(|e| format!("Failed to flush: {}", e))?;
        }
        Ok(())
    }
}

// Global Stream Deck manager instance
lazy_static::lazy_static! {
    pub static ref STREAMDECK_MANAGER: Arc<Mutex<Option<StreamDeckManager>>> = Arc::new(Mutex::new(None));
}


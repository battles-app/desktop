use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serialport::SerialPort;
use hidapi::HidApi;
use std::net::UdpSocket;
use std::time::Duration;
use std::io::Write;
use mdns_sd::{ServiceDaemon, ServiceEvent};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmxDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,       // "enttec_usb", "enttec_ethernet", "opendmx", "dmxis", "artnet", "sacn"
    pub connection_type: String,   // "usb", "ethernet", "network"
    pub port: Option<String>,      // Serial port or IP address
    pub universes: Vec<u8>,        // Supported universes
    pub is_connected: bool,
    pub capabilities: DeviceCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub rdm_support: bool,         // Remote Device Management
    pub multiple_universes: bool,
    pub max_universes: u8,
    pub input_support: bool,       // Can receive DMX input
    pub output_support: bool,      // Can send DMX output
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmxState {
    pub selected_device: Option<DmxDevice>,
    pub available_devices: Vec<DmxDevice>,
    pub universes: HashMap<u8, DmxUniverse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DmxUniverse {
    pub id: u8,
    pub channels: Vec<u8>,
}

impl DmxUniverse {
    pub fn new(id: u8) -> Self {
        Self {
            id,
            channels: vec![0; 512],
        }
    }
}

pub struct DmxManager {
    state: Arc<Mutex<DmxState>>,
    serial_port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
    hid_api: Arc<Mutex<Option<HidApi>>>,
    udp_socket: Arc<Mutex<Option<UdpSocket>>>,
}

impl DmxManager {
    pub fn new() -> Self {
        let mut universes = HashMap::new();
        universes.insert(1, DmxUniverse::new(1));

        Self {
            state: Arc::new(Mutex::new(DmxState {
                selected_device: None,
                available_devices: Vec::new(),
                universes,
            })),
            serial_port: Arc::new(Mutex::new(None)),
            hid_api: Arc::new(Mutex::new(None)),
            udp_socket: Arc::new(Mutex::new(None)),
        }
    }

    /// Comprehensive device scan - USB, Ethernet, and Network protocols
    pub fn scan_devices(&self) -> Result<Vec<DmxDevice>, String> {
        let mut devices = Vec::new();

        println!("[DMX Scan] Starting comprehensive device scan...");

        // 1. Scan USB/Serial devices
        devices.extend(self.scan_usb_devices()?);

        // 2. Scan Ethernet devices (Enttec ODE via mDNS)
        devices.extend(self.scan_ethernet_devices()?);

        // 3. Scan for HID devices (DMXIS)
        devices.extend(self.scan_hid_devices()?);

        // 4. Add network protocols (always available)
        devices.extend(self.get_network_protocols());

        println!("[DMX Scan] Found {} total devices", devices.len());

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            state.available_devices = devices.clone();
        }

        Ok(devices)
    }

    /// Scan for USB/Serial DMX devices (Enttec USB Pro, OpenDMX, etc.)
    fn scan_usb_devices(&self) -> Result<Vec<DmxDevice>, String> {
        let mut devices = Vec::new();

        let ports = serialport::available_ports()
            .map_err(|e| format!("Failed to scan serial ports: {}", e))?;

        for port in ports {
            // Detect Enttec USB Pro / Pro Mk2
            if let serialport::SerialPortType::UsbPort(usb_info) = &port.port_type {
                // Enttec USB Pro VID/PID: 0x0403:0x6001
                // Enttec USB Pro Mk2 VID/PID: 0x0403:0x6015
                if usb_info.vid == 0x0403 {
                    let (device_type, name, universes) = match usb_info.pid {
                        0x6001 => ("enttec_usb", "Enttec USB Pro", vec![1]),
                        0x6015 => ("enttec_usb_mk2", "Enttec USB Pro Mk2", vec![1, 2]),
                        _ => ("enttec_usb", "Enttec USB Device", vec![1]),
                    };

                    devices.push(DmxDevice {
                        id: format!("{}_{}", device_type, port.port_name),
                        name: format!("{} ({})", name, port.port_name),
                        device_type: device_type.to_string(),
                        connection_type: "usb".to_string(),
                        port: Some(port.port_name.clone()),
                        universes,
                        is_connected: false,
                        capabilities: DeviceCapabilities {
                            rdm_support: true,
                            multiple_universes: usb_info.pid == 0x6015,
                            max_universes: if usb_info.pid == 0x6015 { 2 } else { 1 },
                            input_support: true,
                            output_support: true,
                        },
                    });
                    continue;
                }

                // FTDI-based devices (OpenDMX, generic)
                if usb_info.vid == 0x0403 || usb_info.vid == 0x10C4 {
                    devices.push(DmxDevice {
                        id: format!("opendmx_{}", port.port_name),
                        name: format!("DMX USB Interface ({})", port.port_name),
                        device_type: "opendmx".to_string(),
                        connection_type: "usb".to_string(),
                        port: Some(port.port_name.clone()),
                        universes: vec![1],
                        is_connected: false,
                        capabilities: DeviceCapabilities {
                            rdm_support: false,
                            multiple_universes: false,
                            max_universes: 1,
                            input_support: false,
                            output_support: true,
                        },
                    });
                    continue;
                }
            }

            // Generic serial port
            devices.push(DmxDevice {
                id: format!("serial_{}", port.port_name),
                name: format!("Serial DMX ({})", port.port_name),
                device_type: "serial".to_string(),
                connection_type: "usb".to_string(),
                port: Some(port.port_name.clone()),
                universes: vec![1],
                is_connected: false,
                capabilities: DeviceCapabilities {
                    rdm_support: false,
                    multiple_universes: false,
                    max_universes: 1,
                    input_support: false,
                    output_support: true,
                },
            });
        }

        println!("[DMX Scan] Found {} USB/Serial devices", devices.len());
        Ok(devices)
    }

    /// Scan for Ethernet DMX devices (Enttec ODE Mk1/Mk2/Mk3)
    fn scan_ethernet_devices(&self) -> Result<Vec<DmxDevice>, String> {
        let mut devices = Vec::new();

        println!("[DMX Scan] 🔍 Scanning ALL network interfaces for Enttec ODE devices...");

        // Get ALL local network interfaces
        use std::net::{IpAddr, Ipv4Addr};
        
        let mut subnets_to_scan: Vec<String> = Vec::new();
        
        // Method 1: Get all local network interfaces using system commands
        #[cfg(target_os = "windows")]
        {
            if let Ok(output) = std::process::Command::new("ipconfig")
                .output()
            {
                let output_str = String::from_utf8_lossy(&output.stdout);
                for line in output_str.lines() {
                    if line.contains("IPv4 Address") || line.contains("IP Address") {
                        // Extract IP address from line like "   IPv4 Address. . . . . . . . . . . : 192.168.1.116"
                        if let Some(ip_part) = line.split(':').nth(1) {
                            let ip_str = ip_part.trim();
                            if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                                // Skip loopback and APIPA addresses
                                if !ip.is_loopback() && !ip.to_string().starts_with("169.254") {
                                    let octets = ip.octets();
                                    let subnet = format!("{}.{}.{}", octets[0], octets[1], octets[2]);
                                    if !subnets_to_scan.contains(&subnet) {
                                        subnets_to_scan.push(subnet.clone());
                                        println!("[DMX Scan] 📍 Found interface: {}.{} -> will scan subnet {}.x", subnet, octets[3], subnet);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        #[cfg(not(target_os = "windows"))]
        {
            // For Linux/Mac, use ifconfig or ip addr
            if let Ok(output) = std::process::Command::new("sh")
                .arg("-c")
                .arg("ip addr show 2>/dev/null || ifconfig")
                .output()
            {
                let output_str = String::from_utf8_lossy(&output.stdout);
                for line in output_str.lines() {
                    if line.contains("inet ") && !line.contains("inet6") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        for (i, part) in parts.iter().enumerate() {
                            if *part == "inet" && i + 1 < parts.len() {
                                let ip_str = parts[i + 1].split('/').next().unwrap_or("");
                                if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                                    if !ip.is_loopback() && !ip.to_string().starts_with("169.254") {
                                        let octets = ip.octets();
                                        let subnet = format!("{}.{}.{}", octets[0], octets[1], octets[2]);
                                        if !subnets_to_scan.contains(&subnet) {
                                            subnets_to_scan.push(subnet.clone());
                                            println!("[DMX Scan] 📍 Found interface: {}.{} -> will scan subnet {}.x", subnet, octets[3], subnet);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Fallback: If no interfaces found, use the old method
        if subnets_to_scan.is_empty() {
            println!("[DMX Scan] ⚠️  Could not detect network interfaces, using fallback method");
            let local_ip = std::net::UdpSocket::bind("0.0.0.0:0")
                .and_then(|s| {
                    s.connect("8.8.8.8:80")?;
                    s.local_addr()
                })
                .ok()
                .map(|addr| addr.ip().to_string())
                .unwrap_or_else(|| "192.168.1.1".to_string());
            
            if let Some(last_dot) = local_ip.rfind('.') {
                subnets_to_scan.push(local_ip[..last_dot].to_string());
            } else {
                subnets_to_scan.push("192.168.1".to_string());
            }
        }

        println!("[DMX Scan] 🔍 Will scan {} subnet(s): {:?}", subnets_to_scan.len(), subnets_to_scan);

        // Create one socket for all polling (faster)
        match UdpSocket::bind("0.0.0.0:6454") {
            Ok(socket) => {
                println!("[DMX Scan] 🔌 Created UDP socket on port 6454");
                socket.set_read_timeout(Some(Duration::from_millis(50))).ok();
                socket.set_write_timeout(Some(Duration::from_millis(10))).ok();
                socket.set_broadcast(true).ok();
            
            // Build Art-Net Poll packet once
            let mut poll_packet = Vec::new();
            poll_packet.extend_from_slice(b"Art-Net\0"); // ID
            poll_packet.extend_from_slice(&[0x00, 0x20]); // OpCode (ArtPoll = 0x2000)
            poll_packet.extend_from_slice(&[0x00, 0x0e]); // Protocol version 14
            poll_packet.push(0x06); // Flags
            poll_packet.push(0x00); // Priority
            
            // Send polls to full subnet (1-254) to ensure we find the ODE
            let ranges = vec![
                1..255,    // Full subnet scan
            ];
            
            // First, send a broadcast poll
            if let Ok(_) = socket.send_to(&poll_packet, "255.255.255.255:6454") {
                println!("[DMX Scan] 📡 Sent broadcast Art-Net poll to 255.255.255.255");
            }
            
            println!("[DMX Scan] 📤 Sending Art-Net polls to all subnets...");
            let mut sent_count = 0;
            
            // Scan ALL detected subnets
            for subnet in &subnets_to_scan {
                println!("[DMX Scan] 📤 Scanning subnet {}.x...", subnet);
                for range in &ranges {
                    for i in range.clone() {
                        let test_ip = format!("{}.{}", subnet, i);
                        if let Ok(_) = socket.send_to(&poll_packet, format!("{}:6454", test_ip)) {
                            sent_count += 1;
                        }
                    }
                }
            }
            println!("[DMX Scan] 📤 Sent {} Art-Net poll packets total", sent_count);
            
            // Collect responses for 3 seconds (longer for full subnet scan)
            let start = std::time::Instant::now();
            let mut found_ips = std::collections::HashSet::new();
            println!("[DMX Scan] 🔍 Listening for Art-Net responses for 3 seconds...");
            let mut response_count = 0;
            
            // Build list of our own IPs to filter out
            let mut our_ips: Vec<String> = Vec::new();
            for subnet in &subnets_to_scan {
                // Try to get our IP on this subnet by parsing ipconfig output again
                #[cfg(target_os = "windows")]
                {
                    if let Ok(output) = std::process::Command::new("ipconfig").output() {
                        let output_str = String::from_utf8_lossy(&output.stdout);
                        for line in output_str.lines() {
                            if line.contains("IPv4 Address") || line.contains("IP Address") {
                                if let Some(ip_part) = line.split(':').nth(1) {
                                    let ip_str = ip_part.trim();
                                    if ip_str.starts_with(subnet) {
                                        our_ips.push(ip_str.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            println!("[DMX Scan] 🔒 Will filter out our own IPs: {:?}", our_ips);
            
            while start.elapsed() < Duration::from_secs(3) {
                let mut buf = [0u8; 1024];
                if let Ok((len, addr)) = socket.recv_from(&mut buf) {
                    response_count += 1;
                    let ip = addr.ip().to_string();
                    
                    // Skip packets from our own IPs
                    if our_ips.contains(&ip) {
                        println!("[DMX Scan] 🔄 Skipping packet from self ({})", ip);
                        continue;
                    }
                    
                    println!("[DMX Scan] 📥 Received {} bytes from {}", len, addr);
                    
                    // Check if response is ArtPollReply (OpCode 0x2100)
                    if len > 10 && buf[8] == 0x00 && buf[9] == 0x21 {
                        if found_ips.insert(ip.clone()) {
                            println!("[DMX Scan] ✅ Found Art-Net device at {}", ip);
                            devices.push(DmxDevice {
                                id: format!("enttec_ode_mk3_{}", ip),
                                name: format!("Enttec ODE Mk3 ({})", ip),
                                device_type: "enttec_ode_mk3".to_string(),
                                connection_type: "ethernet".to_string(),
                                port: Some(ip.clone()),
                                universes: vec![1, 2],
                                is_connected: false,
                                capabilities: DeviceCapabilities {
                                    rdm_support: true,
                                    multiple_universes: true,
                                    max_universes: 2,
                                    input_support: true,
                                    output_support: true,
                                },
                            });
                        }
                    } else {
                        println!("[DMX Scan] ⚠️  Received non-ArtPollReply packet (OpCode: 0x{:02X}{:02X})", buf[9], buf[8]);
                    }
                }
            }
            
            println!("[DMX Scan] 📊 Response summary: {} total responses, {} unique Art-Net devices found", response_count, found_ips.len());
        }
        Err(e) => {
            println!("[DMX Scan] ❌ Failed to bind UDP socket on port 6454: {}", e);
            println!("[DMX Scan] ⚠️  This could mean:");
            println!("[DMX Scan]    - Port 6454 is already in use by another application");
            println!("[DMX Scan]    - Firewall is blocking the port");
            println!("[DMX Scan]    - Running without Administrator privileges");
        }
    }
        
        println!("[DMX Scan] ✅ Subnet scan complete - found {} devices", devices.len());

        // Use mDNS to discover Enttec ODE devices
        match ServiceDaemon::new() {
            Ok(mdns) => {
                // Browse for Enttec ODE service type
                // Enttec ODE devices advertise via mDNS/Bonjour
                let service_type = "_enttec-ode._tcp.local.";
                
                match mdns.browse(service_type) {
                    Ok(receiver) => {
                        // Wait for responses (timeout after 2 seconds)
                        let timeout = Duration::from_secs(2);
                        let start = std::time::Instant::now();

                        while start.elapsed() < timeout {
                            if let Ok(event) = receiver.recv_timeout(Duration::from_millis(100)) {
                                match event {
                                    ServiceEvent::ServiceResolved(info) => {
                                        // Extract device info from mDNS response
                                        let ip = info.get_addresses().iter()
                                            .find(|addr| addr.is_ipv4())
                                            .map(|addr| addr.to_string());

                                        if let Some(ip_addr) = ip {
                                            // Determine device type from hostname/properties
                                            let hostname = info.get_hostname();
                                            let (device_type, universes) = if hostname.contains("mk3") || hostname.contains("Mk3") {
                                                ("enttec_ode_mk3", vec![1, 2])
                                            } else if hostname.contains("mk2") || hostname.contains("Mk2") {
                                                ("enttec_ode_mk2", vec![1, 2])
                                            } else {
                                                ("enttec_ode", vec![1])
                                            };

                                            devices.push(DmxDevice {
                                                id: format!("enttec_ode_{}", ip_addr),
                                                name: format!("Enttec ODE Mk3 ({})", ip_addr),
                                                device_type: device_type.to_string(),
                                                connection_type: "ethernet".to_string(),
                                                port: Some(ip_addr.clone()),
                                                universes,
                                                is_connected: false,
                                                capabilities: DeviceCapabilities {
                                                    rdm_support: true,
                                                    multiple_universes: true,
                                                    max_universes: 2,
                                                    input_support: true,
                                                    output_support: true,
                                                },
                                            });

                                            println!("[DMX Scan] Found Enttec ODE at {}", ip_addr);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Err(e) => {
                        println!("[DMX Scan] mDNS browse failed: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("[DMX Scan] mDNS initialization failed: {}", e);
            }
        }

        // Also scan local network via UDP broadcast for Enttec ODE
        // Enttec ODE responds to UDP discovery packets on port 3039
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            socket.set_broadcast(true).ok();
            socket.set_read_timeout(Some(Duration::from_secs(1))).ok();

            // Send discovery packet to broadcast address
            let discovery_packet = b"ENTTEC_ODE_DISCOVERY";
            let _ = socket.send_to(discovery_packet, "255.255.255.255:3039");

            // Listen for responses
            let mut buf = [0u8; 1024];
            let start = std::time::Instant::now();

            while start.elapsed() < Duration::from_secs(2) {
                if let Ok((len, addr)) = socket.recv_from(&mut buf) {
                    if len > 0 {
                        let ip_addr = addr.ip().to_string();
                        
                        // Check if we already found this device via mDNS
                        if !devices.iter().any(|d| d.port.as_ref() == Some(&ip_addr)) {
                            devices.push(DmxDevice {
                                id: format!("enttec_ode_{}", ip_addr),
                                name: format!("Enttec ODE ({})", ip_addr),
                                device_type: "enttec_ode".to_string(),
                                connection_type: "ethernet".to_string(),
                                port: Some(ip_addr.clone()),
                                universes: vec![1, 2],
                                is_connected: false,
                                capabilities: DeviceCapabilities {
                                    rdm_support: true,
                                    multiple_universes: true,
                                    max_universes: 2,
                                    input_support: true,
                                    output_support: true,
                                },
                            });

                            println!("[DMX Scan] Found Enttec ODE via UDP at {}", ip_addr);
                        }
                    }
                }
            }
        }

        println!("[DMX Scan] Found {} Ethernet devices", devices.len());
        Ok(devices)
    }

    /// Scan for HID DMX devices (DMXIS, etc.)
    fn scan_hid_devices(&self) -> Result<Vec<DmxDevice>, String> {
        let mut devices = Vec::new();

        let hid_api = HidApi::new()
            .map_err(|e| format!("Failed to initialize HID API: {}", e))?;

        for device in hid_api.device_list() {
            let product_string = device.product_string()
                .unwrap_or("Unknown")
                .to_string();

            // DMXIS VID/PID: typically 0x16D0:0x0B1B or similar
            if product_string.contains("DMX") || product_string.contains("DMXIS") {
                devices.push(DmxDevice {
                    id: format!("dmxis_{:04x}_{:04x}", device.vendor_id(), device.product_id()),
                    name: format!("DMXIS Controller ({})", product_string),
                    device_type: "dmxis".to_string(),
                    connection_type: "usb".to_string(),
                    port: Some(device.path().to_string_lossy().to_string()),
                    universes: vec![1],
                    is_connected: false,
                    capabilities: DeviceCapabilities {
                        rdm_support: false,
                        multiple_universes: false,
                        max_universes: 1,
                        input_support: false,
                        output_support: true,
                    },
                });
            }
        }

        println!("[DMX Scan] Found {} HID devices", devices.len());
        Ok(devices)
    }

    /// Get network protocols (always available)
    fn get_network_protocols(&self) -> Vec<DmxDevice> {
        vec![
            DmxDevice {
                id: "artnet".to_string(),
                name: "Art-Net (Network Broadcast)".to_string(),
                device_type: "artnet".to_string(),
                connection_type: "network".to_string(),
                port: None,
                universes: (0..=255).collect(), // Art-Net supports 256 universes
                is_connected: false,
                capabilities: DeviceCapabilities {
                    rdm_support: true,
                    multiple_universes: true,
                    max_universes: 255,
                    input_support: true,
                    output_support: true,
                },
            },
            DmxDevice {
                id: "sacn".to_string(),
                name: "sACN/E1.31 (Network Multicast)".to_string(),
                device_type: "sacn".to_string(),
                connection_type: "network".to_string(),
                port: None,
                universes: (1..=255).collect(), // sACN supports 63,999 but using u8 practical limit
                is_connected: false,
                capabilities: DeviceCapabilities {
                    rdm_support: false,
                    multiple_universes: true,
                    max_universes: 255, // Practical limit for u8
                    input_support: true,
                    output_support: true,
                },
            },
        ]
    }

    /// Connect to a specific DMX device
    pub fn connect_device(&self, device_id: &str) -> Result<(), String> {
        let device = {
            let state = self.state.lock().unwrap();
            state.available_devices.iter()
                .find(|d| d.id == device_id)
                .cloned()
                .ok_or_else(|| "Device not found".to_string())?
        };

        println!("[DMX] Connecting to device: {} ({})", device.name, device.device_type);

        match device.device_type.as_str() {
            "enttec_usb" | "enttec_usb_mk2" => self.connect_enttec_usb(&device)?,
            "enttec_ode" | "enttec_ode_mk2" | "enttec_ode_mk3" => self.connect_enttec_ode(&device)?,
            "opendmx" | "serial" => self.connect_serial(&device)?,
            "dmxis" => self.connect_hid(&device)?,
            "artnet" => self.connect_artnet()?,
            "sacn" => self.connect_sacn()?,
            _ => return Err("Unknown device type".to_string()),
        }

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            let mut connected_device = device.clone();
            connected_device.is_connected = true;
            state.selected_device = Some(connected_device.clone());

            // Initialize universes for multi-universe devices
            for &univ_id in &connected_device.universes {
                if !state.universes.contains_key(&univ_id) {
                    state.universes.insert(univ_id, DmxUniverse::new(univ_id));
                }
            }
        }

        println!("[DMX] Successfully connected to {}", device.name);
        Ok(())
    }

    /// Connect to Enttec USB Pro/Mk2
    fn connect_enttec_usb(&self, device: &DmxDevice) -> Result<(), String> {
        let port_name = device.port.as_ref()
            .ok_or_else(|| "No port specified".to_string())?;

        crate::file_logger::log(&format!("[DMX] 🔌 Attempting to connect to Enttec USB Pro: {}", port_name));
        crate::file_logger::log(&format!("[DMX]   Using Enttec USB Pro protocol (250000 baud, 8N2)"));

        // CRITICAL: Close any existing connection to this port first
        {
            let mut existing_port = self.serial_port.lock().unwrap();
            if existing_port.is_some() {
                crate::file_logger::log("[DMX]   Closing existing port connection...");
                *existing_port = None; // Drop old port, releasing the handle
                // Give Windows time to release the port
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        let mut port = serialport::new(port_name, 250_000)
            .timeout(Duration::from_millis(500)) // Increased timeout for Windows USB writes
            .data_bits(serialport::DataBits::Eight)
            .stop_bits(serialport::StopBits::Two)
            .parity(serialport::Parity::None)
            .flow_control(serialport::FlowControl::None) // Enttec USB Pro doesn't use flow control
            .open()
            .map_err(|e| {
                let error_msg = format!("{}", e);
                crate::file_logger::log(&format!("[DMX] ❌ Failed to open Enttec USB port: {}", error_msg));
                
                if error_msg.contains("Access is denied") || error_msg.contains("Permission denied") {
                    crate::file_logger::log("[DMX] ⚠️  USB ACCESS DENIED");
                    crate::file_logger::log("[DMX]   This usually means:");
                    crate::file_logger::log("[DMX]   1. The application needs Administrator privileges");
                    crate::file_logger::log("[DMX]   2. Another application is using this device");
                    crate::file_logger::log("[DMX]   3. FTDI/USB drivers are not properly installed");
                    crate::file_logger::log("[DMX]   ");
                    crate::file_logger::log("[DMX]   SOLUTION: Right-click BattlesDesktop and select 'Run as Administrator'");
                    
                    format!(
                        "Access denied to USB port {}. Please:\n\
                        1. Right-click the app and select 'Run as Administrator'\n\
                        2. Close any other DMX software that might be using this device\n\
                        3. Ensure FTDI/USB drivers are installed\n\n\
                        Error details: {}", 
                        port_name, error_msg
                    )
                } else {
                    crate::file_logger::log(&format!("[DMX]   Error details: {}", error_msg));
                    format!("Failed to open Enttec USB ({}): {}", port_name, error_msg)
                }
            })?;

        // CRITICAL: Set DTR/RTS for Enttec USB Pro
        // DTR = Data Terminal Ready, RTS = Request to Send
        // Enttec USB Pro needs both set to true for proper operation
        port.write_data_terminal_ready(true)
            .map_err(|e| format!("Failed to set DTR: {}", e))?;
        port.write_request_to_send(true)
            .map_err(|e| format!("Failed to set RTS: {}", e))?;
        
        crate::file_logger::log("[DMX]   DTR/RTS signals set");
        
        // Give device time to initialize after opening
        std::thread::sleep(std::time::Duration::from_millis(50));

        *self.serial_port.lock().unwrap() = Some(port);
        crate::file_logger::log(&format!("[DMX] ✅ Enttec USB Pro connected successfully: {}", port_name));

        Ok(())
    }

    /// Connect to Enttec ODE (Ethernet)
    fn connect_enttec_ode(&self, device: &DmxDevice) -> Result<(), String> {
        let ip_addr = device.port.as_ref()
            .ok_or_else(|| "No IP address specified".to_string())?;

        println!("[DMX] Enttec ODE connected: {}", ip_addr);

        // Create UDP socket for Enttec ODE communication
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Failed to create UDP socket: {}", e))?;

        *self.udp_socket.lock().unwrap() = Some(socket);

        Ok(())
    }

    /// Connect to serial DMX device (OpenDMX, generic)
    fn connect_serial(&self, device: &DmxDevice) -> Result<(), String> {
        let port_name = device.port.as_ref()
            .ok_or_else(|| "No port specified".to_string())?;

        let port = serialport::new(port_name, 250_000)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Failed to open serial port: {}", e))?;

        *self.serial_port.lock().unwrap() = Some(port);
        println!("[DMX] Serial device connected: {}", port_name);

        Ok(())
    }

    /// Connect to HID DMX device (DMXIS)
    fn connect_hid(&self, device: &DmxDevice) -> Result<(), String> {
        let hid_api = HidApi::new()
            .map_err(|e| format!("Failed to initialize HID API: {}", e))?;

        *self.hid_api.lock().unwrap() = Some(hid_api);
        println!("[DMX] HID device connected: {}", device.name);

        Ok(())
    }

    /// Connect to Art-Net (network)
    fn connect_artnet(&self) -> Result<(), String> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Failed to create UDP socket: {}", e))?;

        socket.set_broadcast(true)
            .map_err(|e| format!("Failed to set broadcast: {}", e))?;

        *self.udp_socket.lock().unwrap() = Some(socket);
        println!("[DMX] Art-Net connected");

        Ok(())
    }

    /// Connect to sACN (network)
    fn connect_sacn(&self) -> Result<(), String> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Failed to create UDP socket: {}", e))?;

        *self.udp_socket.lock().unwrap() = Some(socket);
        println!("[DMX] sACN connected");

        Ok(())
    }

    /// Send DMX data to the selected device
    pub fn send_dmx(&self, universe: u8, start_channel: u16, data: &[u8]) -> Result<(), String> {
        let device = {
            let state = self.state.lock().unwrap();
            state.selected_device.clone()
                .ok_or_else(|| "No device selected".to_string())?
        };

        if !device.is_connected {
            return Err("Device not connected".to_string());
        }

        println!("[DMX] Sending to {}:{} → {:?}", universe, start_channel, data);

        // Update internal DMX buffer
        {
            let mut state = self.state.lock().unwrap();
            if let Some(univ) = state.universes.get_mut(&universe) {
                for (i, &value) in data.iter().enumerate() {
                    let channel_idx = (start_channel as usize + i - 1).min(511);
                    univ.channels[channel_idx] = value;
                }
            }
        }

        // Send to physical device
        match device.device_type.as_str() {
            "enttec_usb" | "enttec_usb_mk2" => self.send_enttec_usb_dmx(universe, &data)?,
            // Enttec ODE devices use Art-Net natively!
            "enttec_ode" | "enttec_ode_mk2" | "enttec_ode_mk3" => {
                println!("[DMX] 📡 ODE Mk3 uses Art-Net protocol - sending Art-Net packet");
                self.send_artnet_dmx_to_ip(&device.port.as_ref().unwrap(), universe, start_channel, &data)?
            }
            "opendmx" | "serial" => self.send_serial_dmx(universe, &data)?,
            "dmxis" => self.send_dmxis_dmx(universe, &data)?,
            "artnet" => self.send_artnet_dmx(universe, start_channel, &data)?,
            "sacn" => self.send_sacn_dmx(universe, start_channel, &data)?,
            _ => return Err("Unsupported device type".to_string()),
        }

        Ok(())
    }

    /// Send DMX via Enttec USB Pro protocol - FIXED packet structure
    fn send_enttec_usb_dmx(&self, universe: u8, _data: &[u8]) -> Result<(), String> {
        // Rate limiting for USB: DMX512 standard is 44Hz max, we'll use 40Hz (25ms)
        use std::time::{Instant, Duration};
        use std::sync::Mutex;
        use std::collections::HashMap;
        
        lazy_static::lazy_static! {
            static ref LAST_USB_SEND: Mutex<HashMap<u8, Instant>> = Mutex::new(HashMap::new());
        }
        
        let now = Instant::now();
        let mut last_sends = LAST_USB_SEND.lock().unwrap();
        
        // Check if enough time has passed since last send (25ms = 40Hz)
        if let Some(last_time) = last_sends.get(&universe) {
            let elapsed = now.duration_since(*last_time);
            if elapsed < Duration::from_millis(25) {
                // Skip this update - too soon
                return Ok(());
            }
        }
        
        // Update last send time
        last_sends.insert(universe, now);
        drop(last_sends); // Release the lock

        let mut port = self.serial_port.lock().unwrap();
        let port = port.as_mut()
            .ok_or_else(|| "Serial port not open".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Enttec USB Pro protocol - CORRECTED VERSION
        // Data length includes DMX Start Code (0x00) + 512 DMX channels = 513 bytes
        let data_len = 513u16;
        
        let mut packet = Vec::with_capacity(518); // 5 header + 513 data + 1 end
        packet.push(0x7E); // Start delimiter
        packet.push(0x06); // Label: Send DMX Packet Request
        packet.push((data_len & 0xFF) as u8); // Data length LSB (513 & 0xFF = 1)
        packet.push(((data_len >> 8) & 0xFF) as u8); // Data length MSB (513 >> 8 = 2)
        packet.push(0x00); // DMX Start Code (this is the FIRST byte of data!)
        packet.extend_from_slice(&full_data); // DMX data (512 channels)
        packet.push(0xE7); // End delimiter

        // Write packet to device
        port.write_all(&packet)
            .map_err(|e| {
                let error_msg = format!("{}", e);
                
                // If access denied, port may be in bad state - log but don't spam
                if error_msg.contains("Access is denied") || error_msg.contains("error 5") {
                    // Only log every 100th error to avoid spam
                    static ERROR_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let count = ERROR_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count % 100 == 0 {
                        crate::file_logger::log(&format!("[DMX] ⚠️  Write error ({}): {}. Port may need reconnection.", count, error_msg));
                    }
                } else {
                    crate::file_logger::log(&format!("[DMX] ❌ Failed to write to Enttec USB: {}", error_msg));
                }
                
                error_msg
            })?;

        // Flush to ensure data is sent immediately
        port.flush()
            .map_err(|e| format!("Failed to flush port: {}", e))?;

        Ok(())
    }

    /// Send DMX via Enttec ODE (Ethernet)
    fn send_enttec_ode_dmx(&self, device: &DmxDevice, universe: u8, _start_channel: u16, _data: &[u8]) -> Result<(), String> {
        let socket = self.udp_socket.lock().unwrap();
        let socket = socket.as_ref()
            .ok_or_else(|| "UDP socket not open".to_string())?;

        let ip_addr = device.port.as_ref()
            .ok_or_else(|| "No IP address".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Enttec ODE protocol (proprietary Enttec Ethernet protocol)
        // Similar to Enttec USB Pro but over UDP
        let mut packet = vec![0x7E]; // Start byte
        packet.push(0x06); // Label
        packet.push((full_data.len() & 0xFF) as u8); // Length LSB
        packet.push(((full_data.len() >> 8) & 0xFF) as u8); // Length MSB
        packet.push(universe); // Universe ID
        packet.extend_from_slice(&full_data);
        packet.push(0xE7); // End byte

        println!("[DMX] 🎯 Enttec ODE: Sending {} bytes to {}:3039 (Universe {})", packet.len(), ip_addr, universe);
        println!("[DMX] 🎯 Enttec ODE: Packet header → 0x7E 0x06 0x{:02X} 0x{:02X} 0x{:02X}", 
                 packet[2], packet[3], packet[4]);
        println!("[DMX] 🎯 Enttec ODE: First 10 DMX channels → {:?}", &full_data[0..10.min(full_data.len())]);

        // Send to Enttec ODE (typically port 3039)
        match socket.send_to(&packet, format!("{}:3039", ip_addr)) {
            Ok(bytes_sent) => {
                println!("[DMX] ✅ Enttec ODE: Sent {} bytes successfully!", bytes_sent);
                Ok(())
            }
            Err(e) => {
                println!("[DMX] ❌ Enttec ODE: Failed to send: {}", e);
                Err(format!("Failed to send to Enttec ODE: {}", e))
            }
        }
    }

    /// Send DMX via basic serial (OpenDMX)
    fn send_serial_dmx(&self, universe: u8, _data: &[u8]) -> Result<(), String> {
        let mut port = self.serial_port.lock().unwrap();
        let port = port.as_mut()
            .ok_or_else(|| "Serial port not open".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Simple DMX packet: Start Code (0x00) + 512 channels
        let mut packet = vec![0x00]; // DMX Start Code
        packet.extend_from_slice(&full_data);

        port.write_all(&packet)
            .map_err(|e| format!("Failed to write to serial device: {}", e))?;

        Ok(())
    }

    /// Send DMX via DMXIS (HID protocol)
    fn send_dmxis_dmx(&self, _universe: u8, _data: &[u8]) -> Result<(), String> {
        // DMXIS uses a proprietary HID protocol
        // This would need the specific DMXIS protocol implementation
        println!("[DMX] DMXIS DMX send not yet implemented");
        Ok(())
    }

    /// Send DMX via Art-Net to a specific IP (for ODE devices)
    fn send_artnet_dmx_to_ip(&self, ip: &str, universe: u8, _start_channel: u16, _data: &[u8]) -> Result<(), String> {
        let socket = self.udp_socket.lock().unwrap();
        let socket = socket.as_ref()
            .ok_or_else(|| "UDP socket not open".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Build Art-Net packet
        let mut packet = Vec::new();
        packet.extend_from_slice(b"Art-Net\0"); // ID
        packet.extend_from_slice(&[0x00, 0x50]); // OpCode (ArtDmx = 0x5000)
        packet.extend_from_slice(&[0x00, 0x0e]); // Protocol version 14
        packet.push(0); // Sequence
        packet.push(0); // Physical port
        packet.push(universe - 1); // Universe LSB (Art-Net universes are 0-indexed!)
        packet.push(0); // Universe MSB
        packet.push(((full_data.len() >> 8) & 0xFF) as u8); // Length MSB
        packet.push((full_data.len() & 0xFF) as u8); // Length LSB
        packet.extend_from_slice(&full_data);

        let dest = format!("{}:6454", ip);
        println!("[DMX] 📡 Art-Net: Sending {} bytes to {} (Universe {})", packet.len(), dest, universe);
        println!("[DMX] 📡 Art-Net: First 20 channels → {:?}", &full_data[0..20.min(full_data.len())]);

        socket.send_to(&packet, &dest)
            .map_err(|e| format!("Failed to send Art-Net packet: {}", e))?;

        println!("[DMX] ✅ Art-Net packet sent successfully to ODE!");
        Ok(())
    }

    /// Send DMX via Art-Net (network protocol broadcast)
    fn send_artnet_dmx(&self, universe: u8, _start_channel: u16, _data: &[u8]) -> Result<(), String> {
        let socket = self.udp_socket.lock().unwrap();
        let socket = socket.as_ref()
            .ok_or_else(|| "UDP socket not open".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Build Art-Net packet
        let mut packet = Vec::new();
        packet.extend_from_slice(b"Art-Net\0"); // ID
        packet.extend_from_slice(&[0x00, 0x50]); // OpCode (ArtDmx = 0x5000)
        packet.extend_from_slice(&[0x00, 0x0e]); // Protocol version 14
        packet.push(0); // Sequence
        packet.push(0); // Physical port
        packet.push(universe); // Universe LSB
        packet.push(0); // Universe MSB
        packet.push(((full_data.len() >> 8) & 0xFF) as u8); // Length MSB
        packet.push((full_data.len() & 0xFF) as u8); // Length LSB
        packet.extend_from_slice(&full_data);

        println!("[DMX] 📡 Art-Net: Broadcasting {} bytes to 255.255.255.255:6454 (Universe {})", packet.len(), universe);
        println!("[DMX] 📡 Art-Net: First 10 channels → {:?}", &full_data[0..10.min(full_data.len())]);

        socket.send_to(&packet, "255.255.255.255:6454")
            .map_err(|e| format!("Failed to send Art-Net packet: {}", e))?;

        println!("[DMX] ✅ Art-Net packet sent successfully");
        Ok(())
    }

    /// Send DMX via sACN/E1.31 (network protocol)
    fn send_sacn_dmx(&self, universe: u8, _start_channel: u16, _data: &[u8]) -> Result<(), String> {
        let socket = self.udp_socket.lock().unwrap();
        let socket = socket.as_ref()
            .ok_or_else(|| "UDP socket not open".to_string())?;

        // Get full 512 channels from state
        let full_data = {
            let state = self.state.lock().unwrap();
            state.universes.get(&universe)
                .map(|u| u.channels.to_vec())
                .unwrap_or_else(|| vec![0; 512])
        };

        // Build sACN/E1.31 packet
        // sACN uses multicast addressing: 239.255.0.x where x = universe number
        let multicast_addr = format!("239.255.0.{}:5568", universe);

        // E1.31 packet structure (simplified)
        let mut packet = Vec::new();
        
        // Root Layer
        packet.extend_from_slice(&[0x00, 0x10]); // Preamble Size
        packet.extend_from_slice(&[0x00, 0x00]); // Post-amble Size
        packet.extend_from_slice(b"ASC-E1.17\0\0\0"); // ACN Packet Identifier
        packet.extend_from_slice(&[0x72, 0x6e]); // Flags and Length
        packet.extend_from_slice(&[0x00, 0x00, 0x00, 0x04]); // Vector (E1.31 Data Packet)
        
        // Source CID (16 bytes UUID)
        packet.extend_from_slice(&[0; 16]);

        // Framing Layer
        packet.extend_from_slice(&[0x72, 0x58]); // Flags and Length
        packet.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]); // Vector (E1.31 Data Packet)
        
        // Source Name (64 bytes)
        let mut source_name = vec![0u8; 64];
        let name = b"Battles Desktop";
        source_name[..name.len()].copy_from_slice(name);
        packet.extend_from_slice(&source_name);

        packet.push(100); // Priority
        packet.extend_from_slice(&[0x00, 0x00]); // Sync Address
        packet.push(0); // Sequence Number
        packet.push(0); // Options
        packet.extend_from_slice(&[0x00, universe]); // Universe

        // DMP Layer
        packet.extend_from_slice(&[0x72, 0x0b]); // Flags and Length
        packet.push(0x02); // Vector (Set Property)
        packet.push(0xa1); // Address Type & Data Type
        packet.extend_from_slice(&[0x00, 0x00]); // First Property Address
        packet.extend_from_slice(&[0x00, 0x01]); // Address Increment
        packet.extend_from_slice(&[0x02, 0x01]); // Property Value Count
        packet.push(0x00); // DMX Start Code
        packet.extend_from_slice(&full_data);

        socket.send_to(&packet, multicast_addr)
            .map_err(|e| format!("Failed to send sACN packet: {}", e))?;

        Ok(())
    }

    /// Get current state
    pub fn get_state(&self) -> DmxState {
        self.state.lock().unwrap().clone()
    }

    /// Disconnect current device
    pub fn disconnect(&self) -> Result<(), String> {
        crate::file_logger::log("[DMX] 🔌 Disconnecting device...");
        
        // Explicitly drop all port handles to release resources
        {
            let mut port = self.serial_port.lock().unwrap();
            if port.is_some() {
                crate::file_logger::log("[DMX]   Closing serial port...");
                *port = None; // Drop the port, triggering proper cleanup
            }
        }
        
        {
            let mut hid = self.hid_api.lock().unwrap();
            if hid.is_some() {
                crate::file_logger::log("[DMX]   Closing HID device...");
                *hid = None;
            }
        }
        
        {
            let mut socket = self.udp_socket.lock().unwrap();
            if socket.is_some() {
                crate::file_logger::log("[DMX]   Closing UDP socket...");
                *socket = None;
            }
        }

        let mut state = self.state.lock().unwrap();
        if let Some(ref mut device) = state.selected_device {
            device.is_connected = false;
        }

        // Small delay to ensure OS releases all resources
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        crate::file_logger::log("[DMX] ✅ Device disconnected successfully");
        Ok(())
    }
}

// Thread-safe singleton
lazy_static::lazy_static! {
    pub static ref DMX_MANAGER: DmxManager = DmxManager::new();
}


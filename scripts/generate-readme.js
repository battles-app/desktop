#!/usr/bin/env node

/**
 * AI-Powered README Generator for Battles.app Desktop Releases
 * 
 * Generates a beautiful, professional README.md for GitHub releases
 * Uses OpenAI GPT-4 to create engaging content based on app features
 */

import { config } from 'dotenv';
import OpenAI from 'openai';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// Load environment variables
config();

// Initialize OpenAI
const openai = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY
});

// Get current version
function getCurrentVersion() {
  const cargoToml = fs.readFileSync(path.join(rootDir, 'Cargo.toml'), 'utf-8');
  const match = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  return match ? match[1] : '1.0.0';
}

// App features and use cases (context for AI)
const appContext = {
  name: 'Battles.app Desktop',
  tagline: 'Pro TikTok Live Utilities - Real-Time Animations, Light Shows, and AI',
  platform: 'Windows 10/11 (64-bit)',
  
  features: [
    {
      name: 'Elgato Stream Deck Integration',
      description: 'Full Stream Deck support with real-time button updates, beautiful branded animations, and instant FX triggering',
      icon: '🎮'
    },
    {
      name: 'Real-Time Animations',
      description: 'Trigger animations, sound effects, and visual FX instantly during TikTok Live streams',
      icon: '🎭'
    },
    {
      name: 'Interactive Light Shows',
      description: 'Synchronized light effects and visual shows for engaging live performances',
      icon: '💡'
    },
    {
      name: 'AI-Powered Tools',
      description: 'Smart automation and AI features for professional streaming',
      icon: '🤖'
    },
    {
      name: 'Beautiful UI',
      description: 'Modern, dark-themed interface with smooth gradients and logo colors',
      icon: '🎨'
    },
    {
      name: 'Auto-Updates',
      description: 'Automatic updates from GitHub releases with cryptographic signature verification',
      icon: '🔄'
    },
    {
      name: 'Battle Board',
      description: 'Global effects library with pre-configured animations and sounds',
      icon: '⚔️'
    },
    {
      name: 'User FX Board',
      description: 'Custom effects and media management for personalized streaming',
      icon: '✨'
    }
  ],
  
  useCases: [
    'TikTok Live streaming with professional effects',
    'Interactive audience engagement with instant FX',
    'Battle and competition streams',
    'Live performances with synchronized effects',
    'Professional content creation',
    'Stream automation and control'
  ],
  
  technicalHighlights: [
    'Native Windows application built with Tauri + Rust',
    'GStreamer for professional video/audio processing',
    'Hardware-accelerated chroma key and compositing',
    'WebSocket real-time communication',
    'Secure auto-updates with cryptographic signing',
    'Stream Deck HID device integration'
  ]
};

// Generate README using OpenAI GPT-4
async function generateReadme(version, changelog = '') {
  console.log('🤖 Generating README with AI...');
  console.log(`Version: ${version}`);
  
  const prompt = `Generate a beautiful, professional README.md for "${appContext.name}" GitHub releases repository.

**Context:**
- Application: ${appContext.name}
- Tagline: ${appContext.tagline}
- Current Version: ${version}
- Platform: ${appContext.platform}
- Status: Closed Beta

**Features:**
${appContext.features.map(f => `- ${f.icon} **${f.name}**: ${f.description}`).join('\n')}

**Use Cases:**
${appContext.useCases.map(u => `- ${u}`).join('\n')}

**Technical Highlights:**
${appContext.technicalHighlights.map(t => `- ${t}`).join('\n')}

**Recent Changes (if any):**
${changelog || 'Initial release with all core features'}

**Requirements:**
1. Eye-catching hero section with ASCII art banner or emoji banner
2. Beautiful badges (version, platform, status)
3. Clear feature highlights with icons
4. Use cases section
5. Installation instructions
6. Links section with:
   - Website: https://battles.app
   - Privacy Policy: https://battles.app/policy
   - Terms of Service: https://battles.app/terms
   - Support: support@battles.app
7. Screenshots/demo section placeholder
8. Beta access information
9. System requirements
10. FAQ section
11. Footer with copyright and branding

**Style Guidelines:**
- Use modern markdown styling
- Include plenty of emojis for visual appeal
- Professional yet friendly tone
- Focus on user benefits, not just features
- Use tables where appropriate
- Add horizontal rules for section separation
- Use blockquotes for important notes
- Include call-to-action buttons/links

Generate ONLY the markdown content, no additional text or explanations.`;

  try {
    const response = await openai.chat.completions.create({
      model: 'gpt-4-turbo-preview',
      messages: [
        {
          role: 'system',
          content: 'You are a professional technical writer and UX designer specializing in creating engaging, beautiful README files for software releases. Create markdown that is visually appealing, informative, and encourages users to try the software.'
        },
        {
          role: 'user',
          content: prompt
        }
      ],
      temperature: 0.8,
      max_tokens: 2500
    });
    
    const readme = response.choices[0].message.content.trim();
    console.log('✅ AI-generated README created!');
    return readme;
    
  } catch (error) {
    console.error('❌ OpenAI API failed:', error.message);
    console.log('Falling back to template README...');
    return generateFallbackReadme(version, changelog);
  }
}

// Fallback README if AI fails
function generateFallbackReadme(version, changelog) {
  return `<div align="center">

# 🎮 Battles.app Desktop

### ${appContext.tagline}

![Version](https://img.shields.io/badge/version-${version}-blue)
![Platform](https://img.shields.io/badge/platform-Windows%2010%2F11-blue)
![Status](https://img.shields.io/badge/status-Closed%20Beta-orange)
![License](https://img.shields.io/badge/license-Proprietary-red)

[Download](#-installation) • [Website](https://battles.app) • [Support](mailto:support@battles.app)

</div>

---

## ✨ Features

${appContext.features.map(f => `### ${f.icon} ${f.name}\n${f.description}\n`).join('\n')}

---

## 🎯 Use Cases

${appContext.useCases.map(u => `- ${u}`).join('\n')}

---

## 📦 Installation

### Windows 10/11 (64-bit)

1. **Download** the latest installer from [Releases](https://github.com/battles-app/desktop-releases/releases/latest)
2. **Run** \`battles.app_${version}_x64-setup.exe\`
3. **Launch** Battles.app Desktop
4. **Connect** your Elgato Stream Deck (optional)
5. **Login** and start streaming!

### System Requirements

- **OS:** Windows 10/11 (64-bit)
- **RAM:** 4GB minimum, 8GB recommended
- **GPU:** DirectX 11 compatible
- **Storage:** 500MB free space
- **Optional:** Elgato Stream Deck (any model)

---

## 🔄 Auto-Updates

Battles.app Desktop includes automatic update functionality:

- ✅ Checks for updates on launch
- ✅ Downloads and installs updates automatically
- ✅ Cryptographically signed for security
- ✅ One-click installation

---

## 🔒 Closed Beta

This software is currently in **closed beta**. 

**Request Access:**
- Visit [https://battles.app](https://battles.app)
- Email: support@battles.app

---

## 📋 What's New

${changelog || 'Initial release with all core features!'}

---

## 🔗 Links

- 🌐 **Website:** [https://battles.app](https://battles.app)
- 📧 **Support:** [support@battles.app](mailto:support@battles.app)
- 📜 **Privacy Policy:** [https://battles.app/policy](https://battles.app/policy)
- 📋 **Terms of Service:** [https://battles.app/terms](https://battles.app/terms)
- 🐛 **Report Issues:** [GitHub Issues](https://github.com/battles-app/desktop-releases/issues)

---

## 💡 FAQ

**Q: Do I need a Stream Deck to use this?**  
A: No! The Stream Deck integration is optional. You can use all features from the desktop app.

**Q: Is this compatible with other streaming software?**  
A: Yes! Battles.app works alongside OBS, Streamlabs, and other streaming tools.

**Q: How do I get beta access?**  
A: Visit [battles.app](https://battles.app) or email support@battles.app to request access.

**Q: Is my data secure?**  
A: Yes. All updates are cryptographically signed, and we follow industry best practices for security.

---

<div align="center">

**BATTLES.app™ © 2025**

Made with ❤️ by the Battles.app team

[Website](https://battles.app) • [Privacy](https://battles.app/policy) • [Terms](https://battles.app/terms)

</div>`;
}

// Main function
async function main() {
  const version = getCurrentVersion();
  
  console.log('');
  console.log('════════════════════════════════════════');
  console.log('  AI README Generator');
  console.log('════════════════════════════════════════');
  console.log('');
  
  // Check for changelog argument
  const changelog = process.argv[2] || '';
  
  // Generate README
  const readme = await generateReadme(version, changelog);
  
  // Save to file
  const outputPath = path.join(rootDir, 'RELEASE_README.md');
  fs.writeFileSync(outputPath, readme, 'utf-8');
  
  console.log('');
  console.log('✅ README saved to:', outputPath);
  console.log('');
}

main().catch(error => {
  console.error('Error:', error.message);
  process.exit(1);
});


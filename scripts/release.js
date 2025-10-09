#!/usr/bin/env node

/**
 * Battles.app Desktop - Release Automation Script
 * 
 * This script automates the release process:
 * 1. Builds the Windows executable
 * 2. Generates AI-powered changelog
 * 3. Creates GitHub release with assets
 * 4. Updates version in all necessary files
 */

import { execSync } from 'child_process';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { config } from 'dotenv';
import OpenAI from 'openai';

// Load environment variables from .env file
config();

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const rootDir = path.resolve(__dirname, '..');

// Load updater private key if it exists
const updaterKeyPath = path.join(rootDir, 'updater-keys.key');
if (fs.existsSync(updaterKeyPath)) {
  const privateKey = fs.readFileSync(updaterKeyPath, 'utf-8').trim();
  process.env.TAURI_SIGNING_PRIVATE_KEY = privateKey;
  console.log('✅ Loaded updater signing key');
}

// Initialize OpenAI
const openai = new OpenAI({
  apiKey: process.env.OPENAI_API_KEY
});

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  pink: '\x1b[38;2;238;43;99m',    // #ee2b63
  yellow: '\x1b[38;2;233;179;32m', // #e9b320
  white: '\x1b[37m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  cyan: '\x1b[36m'
};

const log = {
  info: (msg) => console.log(`${colors.cyan}ℹ${colors.reset} ${msg}`),
  success: (msg) => console.log(`${colors.green}✓${colors.reset} ${msg}`),
  error: (msg) => console.log(`${colors.red}✗${colors.reset} ${msg}`),
  header: (msg) => console.log(`\n${colors.pink}▶${colors.reset} ${colors.yellow}${msg}${colors.reset}\n`),
  battles: () => console.log(`${colors.pink}█${colors.white}█${colors.yellow}█${colors.reset} Battles.app Desktop Release`)
};

// Get current version from Cargo.toml
function getCurrentVersion() {
  const cargoToml = fs.readFileSync(path.join(rootDir, 'Cargo.toml'), 'utf-8');
  const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
  return versionMatch ? versionMatch[1] : '0.1.0';
}

// Increment version
function incrementVersion(version, type = 'patch') {
  const [major, minor, patch] = version.split('.').map(Number);
  
  switch (type) {
    case 'major':
      return `${major + 1}.0.0`;
    case 'minor':
      return `${major}.${minor + 1}.0`;
    case 'patch':
    default:
      return `${major}.${minor}.${patch + 1}`;
  }
}

// Update version in Cargo.toml
function updateCargoVersion(newVersion) {
  const cargoPath = path.join(rootDir, 'Cargo.toml');
  let cargoToml = fs.readFileSync(cargoPath, 'utf-8');
  cargoToml = cargoToml.replace(/^version\s*=\s*"[^"]+"/m, `version = "${newVersion}"`);
  fs.writeFileSync(cargoPath, cargoToml);
}

// Update version in tauri.conf.json5
function updateTauriVersion(newVersion) {
  const tauriPath = path.join(rootDir, 'tauri.conf.json5');
  const tauriContent = fs.readFileSync(tauriPath, 'utf-8');
  
  // Replace version in JSON5 file using regex (preserves formatting and comments)
  const updatedContent = tauriContent.replace(
    /"version":\s*"[\d.]+"/,
    `"version": "${newVersion}"`
  );
  
  fs.writeFileSync(tauriPath, updatedContent, 'utf-8');
}

// Verify version sync between Cargo.toml and tauri.conf.json5
function verifyVersionSync() {
  const cargoPath = path.join(rootDir, 'Cargo.toml');
  const tauriPath = path.join(rootDir, 'tauri.conf.json5');
  
  const cargoContent = fs.readFileSync(cargoPath, 'utf-8');
  const tauriContent = fs.readFileSync(tauriPath, 'utf-8');
  
  const cargoMatch = cargoContent.match(/^version\s*=\s*"([\d.]+)"/m);
  const tauriMatch = tauriContent.match(/"version":\s*"([\d.]+)"/);
  
  const cargoVersion = cargoMatch ? cargoMatch[1] : null;
  const tauriVersion = tauriMatch ? tauriMatch[1] : null;
  
  log.info(`Cargo.toml version: ${cargoVersion}`);
  log.info(`tauri.conf.json5 version: ${tauriVersion}`);
  
  return { cargoVersion, tauriVersion, synced: cargoVersion === tauriVersion };
}

// Generate AI-powered changelog from git commits using OpenAI
async function generateChangelog(fromVersion, toVersion) {
  log.header('Generating AI-Powered Changelog');
  
  try {
    // Get git commits since last tag
    const gitLog = execSync(
      `git log v${fromVersion}..HEAD --pretty=format:"%s" --no-merges`,
      { encoding: 'utf-8' }
    ).trim();
    
    if (!gitLog) {
      log.info('No commits found, using default changelog');
      return '### ✨ New Features\n\n• Initial release\n• Stream Deck integration with beautiful branded animations\n• Dark theme with logo colors\n• Real-time FX control for TikTok Live';
    }
    
    const commits = gitLog.split('\n').filter(line => line.trim());
    
    log.info(`Found ${commits.length} commits since v${fromVersion}`);
    log.info('Sending to OpenAI GPT-4 for professional release notes...');
    
    // Use OpenAI to generate professional release notes
    try {
      const response = await openai.chat.completions.create({
        model: 'gpt-4-turbo-preview',
        messages: [
          {
            role: 'system',
            content: `You are a professional release notes writer for Battles.app Desktop.

Transform git commit messages into minimal, user-friendly release notes.

CRITICAL RULES:
- NEVER use backticks or code blocks in your output
- Keep it minimal - just what changed
- Use plain text for filenames and technical terms
- Use simple bullet points (• symbol)
- Group changes into: ✨ New Features, 🚀 Improvements, 🐛 Bug Fixes
- Each bullet point should be 1 line maximum
- Focus on user benefits, not technical details
- No code examples, no technical jargon
- Use plain text for all content

Example transformation:
"fix streamdeck polling rate" → "• Fixed Stream Deck button responsiveness"
"add loading animation" → "• Added branded loading animation"
"update tauri.conf.json5" → "• Fixed production URL configuration"

Return ONLY the formatted changelog in plain markdown with bullet points. No code blocks, no backticks.`
          },
          {
            role: 'user',
            content: `Generate minimal release notes from these commits:\n\n${commits.join('\n')}`
          }
        ],
        temperature: 0.7,
        max_tokens: 500
      });
      
      const aiChangelog = response.choices[0].message.content.trim();
      log.success('✨ AI-generated changelog created!');
      return aiChangelog;
      
    } catch (aiError) {
      console.log(`${colors.yellow}⚠️  OpenAI API failed: ${aiError.message}${colors.reset}`);
      log.info('Falling back to basic changelog generation...');
      
      // Fallback: Basic categorization if AI fails
      const features = [];
      const fixes = [];
      const improvements = [];
      const other = [];
      
      commits.forEach(commit => {
        const lower = commit.toLowerCase();
        if (lower.includes('feat') || lower.includes('add')) {
          features.push(commit.replace(/^(feat|add)[:\s]*/i, ''));
        } else if (lower.includes('fix') || lower.includes('bug')) {
          fixes.push(commit.replace(/^(fix|bug)[:\s]*/i, ''));
        } else if (lower.includes('improve') || lower.includes('optimize') || lower.includes('perf')) {
          improvements.push(commit.replace(/^(improve|optimize|perf)[:\s]*/i, ''));
        } else {
          other.push(commit);
        }
      });
      
      let changelog = '';
      
      if (features.length > 0) {
        changelog += '### ✨ New Features\n\n';
        features.forEach(feat => changelog += `• ${feat}\n`);
        changelog += '\n';
      }
      
      if (improvements.length > 0) {
        changelog += '### 🚀 Improvements\n\n';
        improvements.forEach(imp => changelog += `• ${imp}\n`);
        changelog += '\n';
      }
      
      if (fixes.length > 0) {
        changelog += '### 🐛 Bug Fixes\n\n';
        fixes.forEach(fix => changelog += `• ${fix}\n`);
        changelog += '\n';
      }
      
      if (other.length > 0 && (features.length === 0 && improvements.length === 0 && fixes.length === 0)) {
        changelog += '### 📝 Changes\n\n';
        other.forEach(change => changelog += `• ${change}\n`);
        changelog += '\n';
      }
      
      return changelog || '• Bug fixes and improvements';
    }
  } catch (error) {
    log.error(`Failed to generate changelog: ${error.message}`);
    return '### 🚀 Improvements\n\n• Bug fixes and performance improvements';
  }
}

// Build the application with production URLs
function buildApp() {
  log.header('Building Application');
  
  // ============================================================================
  // CRITICAL VALIDATION: ABSOLUTE CHECK FOR PRODUCTION URL
  // ============================================================================
  console.log('');
  console.log(`${colors.pink}╔═══════════════════════════════════════════════════════════════╗${colors.reset}`);
  console.log(`${colors.pink}║  ${colors.yellow}CRITICAL: PRODUCTION URL VALIDATION${colors.pink}                        ║${colors.reset}`);
  console.log(`${colors.pink}╚═══════════════════════════════════════════════════════════════╝${colors.reset}`);
  console.log('');
  
  const tauriConfigPath = path.join(rootDir, 'tauri.conf.json5');
  const tauriConfig = fs.readFileSync(tauriConfigPath, 'utf-8');
  const tauriConfigBackup = tauriConfig;
  
  try {
    // ============================================================================
    // STEP 1: CHECK CURRENT STATE - FAIL IF local.battles.app IN WINDOW URL
    // ============================================================================
    const currentUrl = tauriConfig.match(/"url":\s*"([^"]+)"/);
    const currentFrontendDist = tauriConfig.match(/"frontendDist":\s*"([^"]+)"/);
    
    log.info(`Reading tauri.conf.json5...`);
    log.info(`   Window URL: ${currentUrl ? currentUrl[1] : 'NOT FOUND'}`);
    log.info(`   FrontendDist: ${currentFrontendDist ? currentFrontendDist[1] : 'NOT FOUND'}`);
    console.log('');
    
    // CRITICAL CHECK: If window URL contains local.battles.app, FAIL IMMEDIATELY
    if (currentUrl && currentUrl[1].includes('local.battles.app')) {
      console.log(`${colors.red}╔═══════════════════════════════════════════════════════════════╗${colors.reset}`);
      console.log(`${colors.red}║  ⛔ CRITICAL ERROR: LOCAL URL IN PRODUCTION BUILD            ║${colors.reset}`);
      console.log(`${colors.red}╚═══════════════════════════════════════════════════════════════╝${colors.reset}`);
      console.log('');
      console.log(`${colors.red}Window URL contains: ${currentUrl[1]}${colors.reset}`);
      console.log('');
      console.log(`${colors.yellow}This URL MUST be: https://battles.app/${colors.reset}`);
      console.log('');
      throw new Error('FATAL: tauri.conf.json5 has local.battles.app in window URL. Fix this before releasing!');
    }
    
    // ============================================================================
    // STEP 2: ENSURE PRODUCTION URLs (in case they're missing)
    // ============================================================================
    let updatedConfig = tauriConfig;
    
    // Replace any local URLs with production
    updatedConfig = updatedConfig
      .replace(/"url":\s*"https:\/\/local\.battles\.app:3000\/?"/g, '"url": "https://battles.app/"')
      .replace(/"url":\s*"https:\/\/local\.battles\.app\/?"/g, '"url": "https://battles.app/"')
      .replace(/"frontendDist":\s*"https:\/\/battles\.app\/?"/g, '"frontendDist": "../battles.app/dist"')
      .replace(/"frontendDist":\s*"https:\/\/local\.battles\.app:3000\/?"/g, '"frontendDist": "../battles.app/dist"');
    
    // ============================================================================
    // STEP 3: FINAL VALIDATION - VERIFY CORRECT URL
    // ============================================================================
    const finalUrl = updatedConfig.match(/"url":\s*"([^"]+)"/);
    
    if (!finalUrl || !finalUrl[1]) {
      throw new Error('FATAL: Could not find window URL in tauri.conf.json5');
    }
    
    // CHECK: Must contain battles.app
    if (!finalUrl[1].includes('battles.app')) {
      throw new Error(`FATAL: Invalid production URL: ${finalUrl[1]}`);
    }
    
    // CHECK: Must NOT contain local.battles.app
    if (finalUrl[1].includes('local.battles.app')) {
      throw new Error(`FATAL: Window URL still contains local.battles.app: ${finalUrl[1]}`);
    }
    
    // CHECK: Must be exactly https://battles.app/
    if (finalUrl[1] !== 'https://battles.app/') {
      console.log(`${colors.yellow}⚠️  Warning: URL is not exactly "https://battles.app/" but: ${finalUrl[1]}${colors.reset}`);
      console.log(`${colors.yellow}   Proceeding anyway, but this may cause issues.${colors.reset}`);
      console.log('');
    }
    
    // Write the verified config
    fs.writeFileSync(tauriConfigPath, updatedConfig, 'utf-8');
    
    console.log(`${colors.green}╔═══════════════════════════════════════════════════════════════╗${colors.reset}`);
    console.log(`${colors.green}║  ✅ PRODUCTION URL VALIDATED                                  ║${colors.reset}`);
    console.log(`${colors.green}╚═══════════════════════════════════════════════════════════════╝${colors.reset}`);
    console.log('');
    console.log(`${colors.green}✅ Window URL: ${finalUrl[1]}${colors.reset}`);
    console.log(`${colors.cyan}   DevUrl: https://local.battles.app:3000/ (dev only)${colors.reset}`);
    console.log(`${colors.cyan}   FrontendDist: ../battles.app/dist${colors.reset}`);
    console.log('');
    
    // ============================================================================
    // STEP 4: BUILD THE APPLICATION
    // ============================================================================
    log.info('📦 Building Tauri application for Windows (Release mode)...');
    log.info('   ENFORCING: https://battles.app/');
    console.log('');
    
    execSync('bun run tauri build', {
      cwd: rootDir,
      stdio: 'inherit',
      env: {
        ...process.env,
        NODE_ENV: 'production',
        TAURI_ENV_PRODUCTION: 'true',
        TAURI_PRIVATE_KEY: process.env.TAURI_SIGNING_PRIVATE_KEY,
        // Ensure GStreamer is available for DLL bundling
        GSTREAMER_1_0_ROOT_MSVC_X86_64: process.env.GSTREAMER_1_0_ROOT_MSVC_X86_64 || 'E:\\gstreamer\\1.0\\msvc_x86_64'
      }
    });
    
    console.log('');
    log.success('Build completed successfully!');
    
    // ============================================================================
    // STEP 5: POST-BUILD VERIFICATION
    // ============================================================================
    console.log('');
    log.info('🔍 POST-BUILD VERIFICATION: Checking config after build...');
    
    const postBuildConfig = fs.readFileSync(tauriConfigPath, 'utf-8');
    const postBuildUrl = postBuildConfig.match(/"url":\s*"([^"]+)"/);
    
    if (postBuildUrl && postBuildUrl[1].includes('local.battles.app')) {
      console.log('');
      console.log(`${colors.red}╔═══════════════════════════════════════════════════════════════╗${colors.reset}`);
      console.log(`${colors.red}║  ⛔ CRITICAL: CONFIG WAS CHANGED DURING BUILD                ║${colors.reset}`);
      console.log(`${colors.red}╚═══════════════════════════════════════════════════════════════╝${colors.reset}`);
      console.log('');
      throw new Error(`FATAL: Config was modified during build! URL is now: ${postBuildUrl[1]}`);
    }
    
    log.success(`✅ POST-BUILD CHECK PASSED: ${postBuildUrl ? postBuildUrl[1] : 'URL verified'}`);
    console.log('');
    
    // Check if config was changed
    const configChanged = tauriConfig !== updatedConfig;
    
    if (configChanged) {
      // Keep the production config and commit it
      log.info('⚠️  Config was updated to use production URLs');
      log.info('   Committing corrected config to prevent future issues...');
      
      try {
        execSync('git add tauri.conf.json5', { cwd: rootDir, stdio: 'pipe' });
        execSync('git commit -m "fix: ensure tauri.conf.json5 uses production URLs"', { cwd: rootDir, stdio: 'pipe' });
        log.success('✅ Committed production URL config');
        log.info('   To use dev mode locally, run: bun run tauri dev');
      } catch (gitError) {
        log.info('   (Could not auto-commit, but config is correct)');
      }
    } else {
      log.info('✅ Config already correct, no changes needed');
    }
    
    return true;
  } catch (error) {
    // Restore original config on error
    fs.writeFileSync(tauriConfigPath, tauriConfigBackup, 'utf-8');
    log.error('Build failed!');
    log.error(error.message);
    return false;
  }
}

// Find the built executable
function findExecutable() {
  // Try both possible bundle directory locations
  const possibleDirs = [
    path.join(rootDir, 'target', 'release', 'bundle'),
    path.join(rootDir, 'src-tauri', 'target', 'release', 'bundle')
  ];
  
  log.info('Searching for installer in bundle directory...');
  
  for (const bundleDir of possibleDirs) {
    if (!fs.existsSync(bundleDir)) {
      continue;
    }
    
    log.info(`Checking: ${bundleDir}`);
    
    // Look for NSIS installer first (preferred)
    const nsisDir = path.join(bundleDir, 'nsis');
    if (fs.existsSync(nsisDir)) {
      const exeFiles = fs.readdirSync(nsisDir).filter(f => f.endsWith('.exe'));
      if (exeFiles.length > 0) {
        log.success(`Found NSIS installer: ${exeFiles[0]}`);
        return { path: path.join(nsisDir, exeFiles[0]), type: 'exe', name: exeFiles[0] };
      }
    }
    
    // Look for MSI installer as fallback
    const msiDir = path.join(bundleDir, 'msi');
    if (fs.existsSync(msiDir)) {
      const msiFiles = fs.readdirSync(msiDir).filter(f => f.endsWith('.msi'));
      if (msiFiles.length > 0) {
        log.success(`Found MSI installer: ${msiFiles[0]}`);
        return { path: path.join(msiDir, msiFiles[0]), type: 'msi', name: msiFiles[0] };
      }
    }
  }
  
  log.error('No installer found! Expected .exe or .msi in bundle directory');
  log.error(`Checked directories: ${possibleDirs.join(', ')}`);
  return null;
}

// Verify the file is a valid installer (security check)
function verifyInstaller(filePath) {
  const stats = fs.statSync(filePath);
  const filename = path.basename(filePath);
  
  log.info('Verifying installer...');
  log.info(`  File: ${filename}`);
  log.info(`  Size: ${(stats.size / 1024 / 1024).toFixed(2)} MB`);
  
  // Security checks
  if (stats.size < 1024 * 100) { // Less than 100KB is suspicious
    throw new Error('Installer file is too small - possible build failure');
  }
  
  if (stats.size > 1024 * 1024 * 500) { // More than 500MB is suspicious
    throw new Error('Installer file is too large - possible issue');
  }
  
  const ext = path.extname(filePath).toLowerCase();
  if (ext !== '.exe' && ext !== '.msi') {
    throw new Error('Invalid file type - only .exe or .msi allowed');
  }
  
  log.success('Installer verified successfully');
  return true;
}

// Create GitHub release
async function createGitHubRelease(version, changelog, executable) {
  log.header('Creating GitHub Release');
  
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    log.error('GITHUB_TOKEN not found in .env file!');
    log.info('Please add GITHUB_TOKEN to .env file');
    log.info(`\nManual release instructions:`);
    log.info(`1. Go to: https://github.com/battles-app/desktop/releases/new`);
    log.info(`2. Tag: v${version}`);
    log.info(`3. Title: Battles.app Desktop v${version}`);
    log.info(`4. Description:\n${changelog}`);
    log.info(`5. Upload: ${executable.path}`);
    return false;
  }
  
  // Security: Verify installer before uploading
  try {
    verifyInstaller(executable.path);
  } catch (error) {
    log.error(`Installer verification failed: ${error.message}`);
    log.error('Release aborted for security reasons');
    return false;
  }
  
  try {
    // Create tag
    log.info(`Creating tag v${version}...`);
    execSync(`git tag -a v${version} -m "Release v${version}"`, { cwd: rootDir });
    execSync(`git push origin v${version}`, { cwd: rootDir });
    
    // Prepare release notes (NO source code references)
    log.info('Preparing secure release notes...');
    const releaseNotes = `
# 🎮 Battles.app Desktop v${version}

${changelog}

## 📦 Installation

**Windows 10/11 (64-bit)**

1. Download the installer below
2. Run the setup file
3. Launch Battles.app Desktop
4. Connect your Elgato Stream Deck
5. Login and start streaming!

## ⚠️ Closed Beta

This software is in **closed beta**. Access required:
- Request access in the app
- Or visit: https://battles.app

## 🎨 Features

- 🎭 Real-time animations for TikTok Live
- 💡 Interactive light shows and effects
- 🤖 AI-powered streaming tools
- 🎮 Stream Deck integration
- ⚡ Lightning-fast performance

## 🔗 Links

- 🌐 Website: https://battles.app
- 📧 Support: support@battles.app
- 🐛 Issues: https://github.com/battles-app/desktop/issues

---

**⚠️ Security Notice:** This release contains only the compiled installer. No source code is included.

*Made with ❤️ by the Battles.app team*
`;
    
    // Security: Only upload the installer file and updater artifacts
    const installerPath = executable.path;
    const installerName = executable.name;
    
    log.info(`Uploading installer: ${installerName}`);
    log.info(`File size: ${(fs.statSync(installerPath).size / 1024 / 1024).toFixed(2)} MB`);
    
    // Find updater artifacts (for auto-update)
    const bundleDir = path.dirname(installerPath);
    const updaterFiles = [];
    
    // Look for .sig file (same location as installer)
    const sigFile = `${installerPath}.sig`;
    if (fs.existsSync(sigFile)) {
      updaterFiles.push(sigFile);
      log.info(`Found signature file: ${path.basename(sigFile)}`);
    } else {
      console.log(`${colors.yellow}⚠️  Warning: Signature file not found at ${sigFile}${colors.reset}`);
    }
    
    // Generate latest.json for auto-updater
    log.info('Generating latest.json for auto-updater...');
    const latestJsonPath = path.join(rootDir, 'latest.json');
    const signature = fs.existsSync(sigFile) ? fs.readFileSync(sigFile, 'utf-8').trim() : '';
    const latestJson = {
      version: version,
      notes: changelog.replace(/\n/g, ' ').substring(0, 200) + '...',
      pub_date: new Date().toISOString(),
      platforms: {
        'windows-x86_64': {
          signature: signature,
          url: `https://github.com/battles-app/desktop/releases/download/v${version}/${installerName}`
        }
      }
    };
    fs.writeFileSync(latestJsonPath, JSON.stringify(latestJson, null, 2));
    log.success('Generated latest.json');
    updaterFiles.push(latestJsonPath);
    
    // Create file list for upload
    const filesToUpload = [installerPath, ...updaterFiles].map(f => `"${f}"`).join(' ');
    
    // Prepare release notes and repository README
    const releaseNotesPath = path.join(rootDir, 'RELEASE_NOTES.md');
    const repoReadmePath = path.join(rootDir, 'RELEASE_README.md');
    
    // Generate release notes (for GitHub release)
    let releaseNotesContent = '';
    if (fs.existsSync(releaseNotesPath)) {
      releaseNotesContent = fs.readFileSync(releaseNotesPath, 'utf-8');
      log.info('Using AI-generated release notes');
    } else {
      // Fallback to standard release notes
      releaseNotesContent = `# 🎮 Battles.app Desktop v${version}

${changelog}

## 📦 Installation

Download \`${installerName}\` below and run the installer.

**System Requirements:**
- Windows 10/11 (64-bit)
- Elgato Stream Deck (optional)

## ⚠️ Closed Beta

Access required. Request access at: https://battles.app

## 🔗 Links

- 🌐 Website: https://battles.app
- 📧 Support: support@battles.app
- 🐛 Issues: https://github.com/battles-app/desktop/issues

---

**⚠️ Security Notice:** This release contains only the compiled installer. No source code is included.
`;
      fs.writeFileSync(releaseNotesPath, releaseNotesContent);
      log.info('Generated standard release notes');
    }
    
    log.info(`Uploading ${1 + updaterFiles.length} files to battles-app/desktop...`);
    log.info(`  • Installer: ${installerName}`);
    log.info(`  • Signature: ${path.basename(sigFile)}`);
    log.info(`  • Updater manifest: latest.json`);
    
    // Create release on PUBLIC repo (battles-app/desktop) - NO source code!
    execSync(
      `gh release create v${version} ${filesToUpload} --title "Battles.app Desktop v${version}" --notes-file "${releaseNotesPath}" --repo battles-app/desktop`,
      { cwd: rootDir, stdio: 'inherit' }
    );
    
    // Upload README to repository (if AI-generated README exists)
    if (fs.existsSync(repoReadmePath)) {
      log.info('Uploading repository README.md...');
      try {
        // Upload README as an asset to the release (for documentation)
        execSync(
          `gh release upload v${version} "${repoReadmePath}" --repo battles-app/desktop --clobber`,
          { cwd: rootDir }
        );
        log.success('Repository README uploaded as asset');
        
        // Also try to update the repository's README.md file via GitHub API
        try {
          const readmeContent = fs.readFileSync(repoReadmePath, 'utf-8');
          const readmeBase64 = Buffer.from(readmeContent).toString('base64');
          
          // Get current README SHA
          const getShaCmd = `gh api repos/battles-app/desktop/contents/README.md --jq .sha`;
          let sha = '';
          try {
            sha = execSync(getShaCmd, { encoding: 'utf-8' }).trim();
          } catch (e) {
            // README doesn't exist yet
          }
          
          // Update or create README
          const updateCmd = sha 
            ? `gh api repos/battles-app/desktop/contents/README.md -X PUT -f message="docs: update README for v${version}" -f content="${readmeBase64}" -f sha="${sha}"`
            : `gh api repos/battles-app/desktop/contents/README.md -X PUT -f message="docs: create README" -f content="${readmeBase64}"`;
          
          execSync(updateCmd, { cwd: rootDir, stdio: 'pipe' });
          log.success('Repository README.md updated via GitHub API');
        } catch (apiError) {
          log.info('Could not update README via API (may need to set it manually)');
        }
      } catch (error) {
        log.info('Could not upload repository README');
      }
    }
    
    // Upload .github folder contents to repository
    const githubFolder = path.join(rootDir, '.github');
    if (fs.existsSync(githubFolder)) {
      log.info('Uploading .github folder contents...');
      try {
        const files = fs.readdirSync(githubFolder);
        for (const file of files) {
          const filePath = path.join(githubFolder, file);
          const stats = fs.statSync(filePath);
          
          // Only upload files, not directories
          if (stats.isFile()) {
            try {
              const fileContent = fs.readFileSync(filePath);
              const fileBase64 = Buffer.from(fileContent).toString('base64');
              const remotePath = `.github/${file}`;
              
              // Get current file SHA (if exists)
              const getShaCmd = `gh api repos/battles-app/desktop/contents/${remotePath} --jq .sha`;
              let sha = '';
              try {
                sha = execSync(getShaCmd, { encoding: 'utf-8' }).trim();
              } catch (e) {
                // File doesn't exist yet
              }
              
              // Create JSON payload
              const payload = {
                message: sha ? `chore: update ${remotePath} for v${version}` : `chore: add ${remotePath}`,
                content: fileBase64
              };
              
              if (sha) {
                payload.sha = sha;
              }
              
              // Write payload to temp file
              const tempFile = path.join(rootDir, `.temp-${file}.json`);
              fs.writeFileSync(tempFile, JSON.stringify(payload));
              
              // Upload using input file
              const updateCmd = `gh api repos/battles-app/desktop/contents/${remotePath} -X PUT --input "${tempFile}"`;
              execSync(updateCmd, { cwd: rootDir, stdio: 'pipe' });
              
              // Clean up temp file
              fs.unlinkSync(tempFile);
              
              log.success(`  • Uploaded ${remotePath}`);
            } catch (fileError) {
              console.log(`${colors.yellow}⚠${colors.reset} Could not upload .github/${file}: ${fileError.message}`);
            }
          }
        }
        log.success('.github folder contents uploaded');
      } catch (error) {
        log.info('Could not upload .github folder contents');
      }
    }
    
    log.success(`Release v${version} created successfully!`);
    log.info(`View at: https://github.com/battles-app/desktop/releases/tag/v${version}`);
    return true;
  } catch (error) {
    log.error(`Failed to create GitHub release: ${error.message}`);
    return false;
  }
}

// Main release function
async function release() {
  log.battles();
  console.log('');
  
  // Parse arguments
  const args = process.argv.slice(2);
  const versionType = args[0] || 'patch'; // major, minor, or patch
  
  if (!['major', 'minor', 'patch'].includes(versionType)) {
    log.error(`Invalid version type: ${versionType}`);
    log.info('Usage: bun run release [major|minor|patch]');
    process.exit(1);
  }
  
  // Get current version and calculate new version
  const currentVersion = getCurrentVersion();
  const newVersion = incrementVersion(currentVersion, versionType);
  
  log.header('Release Information');
  
  // Verify version sync before starting
  const versionCheck = verifyVersionSync();
  
  console.log(`${colors.cyan}📦 Current State:${colors.reset}`);
  console.log(`   Cargo.toml:         ${versionCheck.cargoVersion}`);
  console.log(`   tauri.conf.json5:   ${versionCheck.tauriVersion}`);
  console.log(`   Synced:             ${versionCheck.synced ? '✅' : '❌'}`);
  console.log('');
  console.log(`${colors.green}🚀 Release Plan:${colors.reset}`);
  console.log(`   From:               ${currentVersion}`);
  console.log(`   To:                 ${newVersion}`);
  console.log(`   Type:               ${versionType}`);
  console.log('');
  
  if (!versionCheck.synced) {
    console.log(`${colors.yellow}⚠️  Versions will be synchronized to ${newVersion}${colors.reset}`);
    console.log('');
  }
  
  // Confirm
  if (!process.env.CI) {
    const readline = await import('readline');
    const rl = readline.createInterface({
      input: process.stdin,
      output: process.stdout
    });
    
    await new Promise((resolve) => {
      rl.question(`${colors.yellow}Continue with release v${newVersion}? (y/N): ${colors.reset}`, (answer) => {
        rl.close();
        if (answer.toLowerCase() !== 'y') {
          log.info('Release cancelled');
          process.exit(0);
        }
        resolve();
      });
    });
  }
  
  // Update versions
  log.header('Updating Versions');
  updateCargoVersion(newVersion);
  log.success('Updated Cargo.toml');
  updateTauriVersion(newVersion);
  log.success('Updated tauri.conf.json5');
  
  // Generate changelog
  const changelog = await generateChangelog(currentVersion, newVersion);
  log.success('Generated changelog');
  
  // Generate AI content (release notes + repository README)
  log.header('Generating AI Content');
  try {
    execSync(`node scripts/generate-readme.js "${changelog}"`, {
      cwd: rootDir,
      stdio: 'inherit'
    });
    log.success('AI-powered content generated');
  } catch (error) {
    log.error('Failed to generate AI content (continuing anyway)');
  }
  
  // Clean old builds to prevent version mismatches
  log.header('Cleaning Old Builds');
  try {
    const targetDir = path.join(rootDir, 'target', 'release', 'bundle');
    if (fs.existsSync(targetDir)) {
      log.info('Removing old build artifacts...');
      fs.rmSync(targetDir, { recursive: true, force: true });
      log.success('Old builds cleaned');
    } else {
      log.info('No old builds to clean');
    }
  } catch (error) {
    log.info('Could not clean old builds (continuing anyway)');
  }
  
  // Build
  if (!buildApp()) {
    process.exit(1);
  }
  
  // Find executable
  const executable = findExecutable();
  if (!executable) {
    log.error('Could not find built executable!');
    log.error('Make sure the build completed successfully');
    process.exit(1);
  }
  log.success(`Found installer: ${executable.name}`);
  log.info(`Location: ${executable.path}`);
  
  // CRITICAL: Verify the executable version matches the release version
  const executableVersion = executable.name.match(/battles\.app_([0-9.]+)_x64/);
  if (executableVersion && executableVersion[1] !== newVersion) {
    console.log('');
    console.log(`${colors.red}╔═══════════════════════════════════════════════════════════════╗${colors.reset}`);
    console.log(`${colors.red}║  ⛔ VERSION MISMATCH DETECTED                                ║${colors.reset}`);
    console.log(`${colors.red}╚═══════════════════════════════════════════════════════════════╝${colors.reset}`);
    console.log('');
    console.log(`${colors.red}Executable file:     ${executable.name}${colors.reset}`);
    console.log(`${colors.red}Executable version:  ${executableVersion[1]}${colors.reset}`);
    console.log(`${colors.yellow}Expected version:    ${newVersion}${colors.reset}`);
    console.log('');
    log.error('FATAL: The built executable version does not match the release version!');
    log.error('This indicates a build issue. Please run "bun run release" again.');
    process.exit(1);
  }
  log.success(`✅ Version verified: ${newVersion} matches executable`);
  
  // Commit version changes to PRIVATE repo only (NOT to battles-app/desktop)
  log.header('Committing Version Changes Locally');
  try {
    execSync('git add Cargo.toml tauri.conf.json5 Cargo.lock', { cwd: rootDir });
    execSync(`git commit -m "chore: bump version to ${newVersion}"`, { cwd: rootDir });
    
    // IMPORTANT: Check if we're on the private repo before pushing
    const remoteUrl = execSync('git config --get remote.origin.url', { cwd: rootDir, encoding: 'utf-8' }).trim();
    if (remoteUrl.includes('gkarmas/battles-desktop')) {
      execSync('git push', { cwd: rootDir });
      log.success('Committed and pushed version changes to PRIVATE repo');
    } else {
      log.info('Skipping git push - not on private repository');
      log.info('Version changes committed locally only');
    }
  } catch (error) {
    log.error('Failed to commit changes (this is okay if no changes)');
  }
  
  // Create GitHub release (to battles-app/desktop via API - NO source code!)
  await createGitHubRelease(newVersion, changelog, executable);
  
  log.header('Release Complete! 🎉');
  log.success(`Version ${newVersion} has been released!`);
  console.log('');
}

// Run
release().catch((error) => {
  log.error(`Release failed: ${error.message}`);
  process.exit(1);
});


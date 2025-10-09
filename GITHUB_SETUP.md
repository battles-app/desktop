# GitHub Repository Setup Guide

## 📦 Create the Release Repository

### 1. Create Repository on GitHub

Go to https://github.com/organizations/battles-app/repositories/new

**Settings:**
- **Owner**: `battles-app`
- **Repository name**: `desktop-releases`
- **Description**: `🎮 Professional live streaming software with Elgato Stream Deck integration - Windows Desktop App (Closed Beta)`
- **Visibility**: ✅ Public
- **Initialize**: ❌ Do NOT initialize with README (we have our own)

### 2. Repository Settings

After creating, go to Settings:

#### About Section (top right)
- **Website**: `https://battles.app`
- **Topics**: `streamdeck`, `elgato`, `streaming`, `desktop-app`, `windows`, `tauri`, `rust`
- **Releases**: ✅ Check
- **Packages**: ❌ Uncheck

#### Description
```
🎮 Battles.app Desktop - Professional live streaming software with Elgato Stream Deck integration. Beautiful branded animations, real-time FX control, lightning-fast performance. Windows only. Closed Beta.
```

#### Features (in Settings)
- ✅ Issues
- ❌ Projects  
- ❌ Wiki
- ✅ Discussions (optional)

### 3. Clone and Setup

```bash
# Clone the new repository
git clone https://github.com/battles-app/desktop-releases.git
cd desktop-releases

# Copy the beautiful README
cp ../battlesDesktop/RELEASE_README.md README.md

# Copy GitHub templates (create .github directory structure first)
mkdir -p .github/ISSUE_TEMPLATE
# Copy issue templates manually or via script

# Create initial commit
git add .
git commit -m "docs: initial release repository setup with branded README"
git push origin main
```

### 4. Configure GitHub Token

For automated releases, you need a GitHub Personal Access Token:

1. Go to https://github.com/settings/tokens
2. Click "Generate new token (classic)"
3. Name: `Battles Desktop Releases`
4. Scopes:
   - ✅ `repo` (all)
   - ✅ `write:packages`
5. Generate and copy the token
6. Save it securely

**Set environment variable:**
```powershell
# Windows PowerShell
$env:GITHUB_TOKEN = "ghp_your_token_here"

# Or permanently:
[System.Environment]::SetEnvironmentVariable('GITHUB_TOKEN', 'ghp_your_token_here', 'User')
```

### 5. Install GitHub CLI (gh)

Download from: https://cli.github.com/

Or install via winget:
```powershell
winget install GitHub.cli
```

Authenticate:
```bash
gh auth login
```

## 🚀 First Release

Once setup is complete:

```bash
cd battlesDesktop

# Make sure you're on main branch with clean working directory
git status

# Create first release (will be v0.1.0 -> v0.1.1 or as specified in Cargo.toml)
bun run release

# Or specify version bump type:
bun run release patch   # 0.1.0 -> 0.1.1
bun run release minor   # 0.1.0 -> 0.2.0
bun run release major   # 0.1.0 -> 1.0.0
```

### What the Release Script Does:

1. ✅ Reads current version from `Cargo.toml`
2. ✅ Calculates new version based on type (major/minor/patch)
3. ✅ Updates `Cargo.toml` and `tauri.conf.json`
4. ✅ Generates AI-powered changelog from git commits
5. ✅ Builds Windows executable with `bun tauri build`
6. ✅ Finds the built installer (MSI or EXE)
7. ✅ Commits version bump
8. ✅ Creates git tag `vX.Y.Z`
9. ✅ Pushes tag to GitHub
10. ✅ Creates GitHub Release with:
    - Beautiful branded description
    - Installation instructions
    - Changelog
    - System requirements
    - Closed beta notice
11. ✅ Uploads executable to release

## 📝 Commit Message Format

For best AI-generated changelogs, use conventional commits:

```bash
# Features
git commit -m "feat: add Stream Deck XL support"
git commit -m "add: beautiful loading animation with logo"

# Fixes
git commit -m "fix: animation frame rate optimization"
git commit -m "bug: resolve USB bandwidth issues"

# Improvements
git commit -m "improve: faster button rendering"
git commit -m "optimize: reduce memory usage"
git commit -m "perf: 5x faster animation loop"

# Others
git commit -m "chore: bump version to 0.2.0"
git commit -m "docs: update README with troubleshooting"
```

The release script automatically categorizes these into:
- ✨ **New Features**
- 🚀 **Improvements**
- 🐛 **Bug Fixes**

## 🎨 Branding

The repository uses Battles.app brand colors:
- **Pink**: `#ee2b63` (Primary brand color)
- **White**: `#ffffff` (Secondary)
- **Yellow**: `#e9b320` (Accent)

These colors are used in:
- README badges
- Console output
- Release notes
- Issue templates

## 📞 Support

For issues with the release process:
- Check console logs during `bun run release`
- Verify GitHub token permissions
- Ensure GitHub CLI (`gh`) is installed and authenticated
- Contact @gkarmas if repository access issues

---

Made with ❤️ by the Battles.app team


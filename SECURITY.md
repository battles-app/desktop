# 🔒 Security Policy

## Release Security

### What Gets Released

**✅ ONLY the installer is released publicly:**
- Windows installer (`.exe` or `.msi`)
- Compiled binary only
- No source code
- No configuration files
- No development files

**❌ NEVER released:**
- Source code
- `.env` files
- API keys or tokens
- Development dependencies
- Build artifacts (except final installer)
- Internal configuration

### Build Verification

Every release goes through security checks:

1. **File Type Validation**
   - Only `.exe` or `.msi` files allowed
   - File extension strictly verified

2. **Size Verification**
   - Minimum 100 KB (prevents empty/corrupted files)
   - Maximum 500 MB (prevents bloated builds)

3. **Integrity Checks**
   - File exists and is readable
   - Proper permissions set
   - Valid installer format

### Release Process

1. **Build** - Application compiled in release mode
2. **Verify** - Security checks run automatically
3. **Tag** - Git tag created (version only, no code)
4. **Upload** - ONLY installer uploaded to GitHub releases
5. **Publish** - Release made public

### Token Security

- GitHub token stored in `.env` (never committed)
- `.env` file in `.gitignore`
- Token has minimal required permissions:
  - `repo` - for releases only
  - `write:packages` - for release assets

### Repository Separation

- **Source Code**: Private repository (not public)
- **Releases**: Public repository at `battles-app/desktop-releases`
  - Contains ONLY:
    - Release binaries
    - Release notes
    - Installation instructions
    - Documentation
  - Does NOT contain:
    - Source code
    - Build scripts
    - Environment files
    - Development tools

## Reporting Security Issues

If you discover a security vulnerability:

1. **DO NOT** create a public issue
2. Email: security@battles.app
3. Include:
   - Description of vulnerability
   - Steps to reproduce
   - Potential impact
   - Your contact information

We'll respond within 48 hours and work with you to resolve the issue.

## Security Best Practices

### For Users

- ✅ Download only from official GitHub releases
- ✅ Verify the repository: `battles-app/desktop-releases`
- ✅ Check file size matches release notes
- ❌ Don't download from unofficial sources
- ❌ Don't bypass Windows security warnings without verification

### For Developers

- ✅ Keep `.env` file secure and private
- ✅ Never commit tokens or API keys
- ✅ Use minimal required permissions
- ✅ Verify releases before publishing
- ❌ Never include source code in releases
- ❌ Never share access tokens

## Closed Beta Access

This software requires beta access:

- Access is granted per-user
- Request via https://battles.app
- Email verification required
- Terms of Service acceptance required

Unauthorized use or distribution is prohibited.

---

**Last Updated**: 2025-01-09

**Contact**: security@battles.app


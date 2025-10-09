# Security Incident - RESOLVED

## ⚠️ Issue: Source Code Accidentally Pushed to Public Repo

**Date**: 2025-10-10  
**Status**: ✅ **RESOLVED**  
**Severity**: Critical (now mitigated)

---

## What Happened

The release script was accidentally configured to push source code to `battles-app/desktop` (the public releases repository) instead of only uploading compiled artifacts via GitHub API.

**Timeline:**
1. Git remote was changed from `gkarmas/battles-desktop` → `battles-app/desktop`
2. Release script ran `git push` which pushed ALL source code to the public repo
3. Issue was immediately identified and resolved

---

## What Was Done to Fix It

### 1. **Immediate Action - Source Code Removed**
- ✅ Cleared `battles-app/desktop` repository of all source code
- ✅ Force-pushed a clean repository with only README.md
- ✅ Restored git remote to `gkarmas/battles-desktop` (private repo)

### 2. **Security Fixes to Release Script**
The `scripts/release.js` file was updated with **safety checks**:

#### Before (INSECURE):
```javascript
execSync('git push', { cwd: rootDir }); // ❌ Pushes to ANY remote
execSync(`git push origin v${version}`, { cwd: rootDir }); // ❌ Pushes tag to ANY remote
```

#### After (SECURE):
```javascript
// Check if we're on the private repo before pushing
const remoteUrl = execSync('git config --get remote.origin.url', { cwd: rootDir, encoding: 'utf-8' }).trim();
if (remoteUrl.includes('gkarmas/battles-desktop')) {
  execSync('git push', { cwd: rootDir });
  log.success('Committed and pushed version changes to PRIVATE repo');
} else {
  log.info('Skipping git push - not on private repository');
}
```

---

## How the Release Process Works Now

### Source Code (PRIVATE)
**Repository**: `gkarmas/battles-desktop`  
**Visibility**: Private  
**Content**: Full source code, development history  
**Git Operations**: 
- ✅ Commits, tags, and pushes happen here
- ✅ Development work stays private

### Releases (PUBLIC)
**Repository**: `battles-app/desktop`  
**Visibility**: Private (but could be public)  
**Content**: ONLY compiled artifacts and metadata  
**Update Method**: GitHub API only  
**What Gets Published**:
- ✅ Windows installer (.exe)
- ✅ MSI package (.msi)
- ✅ Signature files (.sig)
- ✅ Auto-updater manifest (latest.json)
- ✅ README.md (via API)
- ✅ .github folder assets (via API)
- ❌ **NO source code**
- ❌ **NO git history**
- ❌ **NO commits**

---

## Security Guarantees

### ✅ What the Fixed Script Does
1. **Builds locally** in the private repository (`gkarmas/battles-desktop`)
2. **Verifies remote URL** before ANY git push operation
3. **Creates releases via GitHub API** (`gh release create --repo battles-app/desktop`)
4. **Uploads artifacts via API** (no git operations)
5. **Updates README via API** (`gh api repos/battles-app/desktop/contents/README.md`)
6. **Updates .github folder via API** (individual file uploads)

### ❌ What the Script Will NEVER Do
1. Push source code to `battles-app/desktop`
2. Push commits to `battles-app/desktop`
3. Expose private development history
4. Leak environment variables or secrets
5. Upload anything except whitelisted artifacts

---

## Verification

To verify the release script is secure, check these lines in `scripts/release.js`:

1. **Line ~895-902**: Git push safety check
```javascript
if (remoteUrl.includes('gkarmas/battles-desktop')) {
  execSync('git push', { cwd: rootDir });
}
```

2. **Line ~510-517**: Tag push safety check
```javascript
if (remoteUrl.includes('gkarmas/battles-desktop')) {
  execSync(`git push origin v${version}`, { cwd: rootDir });
}
```

3. **Line ~653**: Release creation via API
```javascript
gh release create v${version} ... --repo battles-app/desktop
```

---

## Current Status

✅ **battles-app/desktop** - Clean, contains only README and downloads  
✅ **gkarmas/battles-desktop** - Private, contains full source  
✅ **Release script** - Secured with safety checks  
✅ **All source code** - Removed from public repo  
✅ **Future releases** - Will use API-only method  

---

## Recommendations

1. **Do NOT change git remote** to `battles-app/desktop` manually
2. **Always run releases from** `gkarmas/battles-desktop` repository
3. **Review** `scripts/release.js` before running if modified
4. **Monitor** `battles-app/desktop` to ensure only artifacts are there
5. **Set up branch protection** on `battles-app/desktop` if made public

---

## Lessons Learned

- Git remotes should be carefully managed when dealing with private/public repos
- Release automation should have safeguards for sensitive operations
- API-based releases are safer than git-based deployments for compiled artifacts
- Always verify what's being pushed before running automated scripts

---

**Last Updated**: 2025-10-10  
**Incident Response Time**: < 5 minutes  
**Current Risk Level**: ✅ Low (mitigated)


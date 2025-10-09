# Stream Deck Access Token Implementation

## What Changed

Added Directus access token authentication to Stream Deck image URLs, just like the OBS overlay system.

## Changes Made

### 1. Frontend (`battles.app/components/DashboardView.vue`)

**Added access token fetching:**
```typescript
// Get authentication token for Directus access
let accessToken = null
try {
  const authResponse = await $fetch('/api/auth/token')
  if (authResponse.accessToken) {
    accessToken = authResponse.accessToken
    console.log('[Stream Deck] Got access token for image downloads')
  }
} catch (authError) {
  console.warn('[Stream Deck] Could not get access token:', authError)
}
```

**Created URL builder helper:**
```typescript
const buildImageUrl = (fileId: string) => {
  const url = new URL(`${baseUrl}/directus-assets/${fileId}`)
  url.searchParams.set('width', '96')
  url.searchParams.set('height', '96')
  url.searchParams.set('fit', 'cover')
  url.searchParams.set('format', 'jpg')
  if (accessToken) {
    url.searchParams.set('access_token', accessToken)
  }
  return url.toString()
}
```

**URL format now:**
```
https://local.battles.app:3000/directus-assets/{file-id}?width=96&height=96&fit=cover&format=jpg&access_token=act.xxx...
```

### 2. Backend (`battles.app/server/routes/directus-assets/[...path].get.ts`)

**Added query parameter forwarding:**
```typescript
// Get query parameters from the request
const query = getQuery(event)

// Build target URL with query parameters
const targetUrl = new URL(`${directusUrl}/assets/${path}`)

// Forward all query parameters to Directus (width, height, fit, format, access_token, etc.)
Object.entries(query).forEach(([key, value]) => {
  if (value !== undefined && value !== null) {
    targetUrl.searchParams.set(key, String(value))
  }
})
```

Now the proxy **forwards all query parameters** including `access_token` to Directus!

## How It Works

### Flow:

1. **Frontend** fetches access token from `/api/auth/token`
2. **Frontend** builds image URLs with:
   - Transformation params (`width=96&height=96&fit=cover&format=jpg`)
   - Access token (`access_token=act.xxx...`)
3. **Rust backend** downloads from these URLs
4. **Nuxt proxy** forwards all params to Directus
5. **Directus** validates token and generates thumbnail
6. **Image** is cached and displayed on Stream Deck

### Security:

- ✅ **HTTPS only** - Access tokens transmitted securely
- ✅ **User's own token** - Each user accesses only their files
- ✅ **No admin token exposed** - User tokens have limited permissions
- ✅ **Token expiration** - Tokens expire and refresh automatically

## Benefits

- ✅ **Secure access** - Files protected by Directus authentication
- ✅ **Private files** - Users can only access their own FX files
- ✅ **Production ready** - Same auth system as OBS overlay
- ✅ **Token refresh** - Uses existing session management

## Testing

1. **Clear cache:**
   ```powershell
   .\clear-streamdeck-cache.ps1
   ```

2. **Restart app:**
   ```powershell
   bun run tauri dev
   ```

3. **Watch for logs:**
   ```
   [Stream Deck] Got access token for image downloads
   [Stream Deck] Downloading image from: https://local.battles.app:3000/directus-assets/...&access_token=act.xxx
   [Stream Deck] Content-Type: image/jpeg
   [Stream Deck] ✅ Cached x2 (12,456 bytes)
   ```

## Example URL

**Before:**
```
https://local.battles.app:3000/directus-assets/f1bd0750-f531-4712-9fda-8c12085cd63e?width=96&height=96&fit=cover&format=jpg
```

**After:**
```
https://local.battles.app:3000/directus-assets/f1bd0750-f531-4712-9fda-8c12085cd63e?width=96&height=96&fit=cover&format=jpg&access_token=act.tBj7Ky6CeId0If2N39MBpLpKQD8nxYIVMaj6xr0R7VRUT7zXppdFgK77SKxC%214969.e1
```

## Notes

- **Automatic fallback**: If token fetch fails, URLs still work (for public files)
- **Same as OBS**: Uses identical auth mechanism to TV monitor/OBS overlay
- **Cache works**: Cached thumbnails don't need re-authentication
- **Token in URL**: Safe over HTTPS, same as OBS overlay URLs

## Troubleshooting

**If images don't download:**
1. Check console for "[Stream Deck] Got access token" log
2. If missing, verify user is logged in
3. Check `/api/auth/token` returns `accessToken`

**If 401 errors:**
- Token might be expired, restart app to refresh session
- User might not have access to files

**If 404 errors:**
- File ID might be invalid
- Directus file might have been deleted


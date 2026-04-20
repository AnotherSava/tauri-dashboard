---
name: configuring-tauri-capabilities
description: "Creates and edits Tauri capability JSON files, configures plugin permissions and per-window access control, and sets up platform-specific security boundaries. Use when working with capability.json, Tauri allowlist, IPC permissions, plugin access, Tauri security config, or per-window access control."
---

# Tauri Capabilities Configuration

## Workflow

Follow this sequence when configuring capabilities:

1. **Create capability file** in `src-tauri/capabilities/` (JSON or TOML)
2. **Define permissions** — assign plugin and core permissions to target windows
3. **Build project** — run `cargo tauri build` or `cargo tauri dev` to generate schemas
4. **Verify** — confirm denied APIs return errors in the frontend console

**Validation checkpoints:**
- `cargo tauri build` succeeds without capability errors
- Permitted APIs respond correctly from the target window
- Denied APIs return permission errors in the frontend console
- Window labels in capabilities match labels in Rust window creation code

## Capability File Structure

Capability files reside in `src-tauri/capabilities/` and use JSON or TOML format. All files in this directory are automatically enabled unless explicitly configured in `tauri.conf.json`.

| Field | Required | Description |
|-------|----------|-------------|
| `identifier` | Yes | Unique capability name |
| `description` | No | Purpose explanation |
| `windows` | Yes | Target window labels (supports wildcards) |
| `permissions` | Yes | Array of allowed/denied operations |
| `platforms` | No | Target platforms (linux, macOS, windows, iOS, android) |
| `remote` | No | Remote URL access configuration |
| `$schema` | No | Reference to generated schema for IDE support |

## Basic Capability Example

Create `src-tauri/capabilities/main.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-capability",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:path:default",
    "core:event:default",
    "core:window:default",
    "core:app:default",
    "core:resources:default",
    "core:menu:default",
    "core:tray:default"
  ]
}
```

## Default vs Explicit Capability Loading

To explicitly control which capabilities are active, configure them in `tauri.conf.json`:

```json
{
  "app": {
    "security": {
      "capabilities": ["main-capability", "editor-capability"]
    }
  }
}
```

When explicitly configured, only the listed capabilities apply.

## Configuration Methods

### Method 1: Separate Files (Recommended)

Store individual capability files in the capabilities directory:

```
src-tauri/
  capabilities/
    main.json
    editor.json
    settings.json
```

### Method 2: Inline Definition

Embed capabilities directly in `tauri.conf.json`:

```json
{
  "app": {
    "security": {
      "capabilities": [
        {
          "identifier": "my-capability",
          "windows": ["*"],
          "permissions": ["fs:default", "core:window:default"]
        }
      ]
    }
  }
}
```

### Method 3: Mixed Approach

Combine file-based and inline capabilities:

```json
{
  "app": {
    "security": {
      "capabilities": [
        {
          "identifier": "inline-capability",
          "windows": ["*"],
          "permissions": ["fs:default"]
        },
        "file-based-capability"
      ]
    }
  }
}
```

## Per-Window Capabilities

Assign different permissions to different windows using window labels:

### Single Window

```json
{
  "identifier": "main-capability",
  "windows": ["main"],
  "permissions": ["core:window:default", "fs:default"]
}
```

### Multiple Specific Windows

```json
{
  "identifier": "editor-capability",
  "windows": ["editor", "preview"],
  "permissions": ["fs:read-files", "core:event:default"]
}
```

### Wildcard (All Windows)

```json
{
  "identifier": "global-capability",
  "windows": ["*"],
  "permissions": ["core:event:default"]
}
```

### Pattern Matching

```json
{
  "identifier": "dialog-capability",
  "windows": ["dialog-*"],
  "permissions": ["core:window:allow-close"]
}
```

## Permission Syntax

Permissions follow a naming convention:

| Pattern | Description |
|---------|-------------|
| `<plugin>:default` | Default permission set for a plugin |
| `<plugin>:allow-<command>` | Allow a specific command |
| `<plugin>:deny-<command>` | Deny a specific command |

### Core Permissions

```json
{
  "permissions": [
    "core:path:default",
    "core:event:default",
    "core:window:default",
    "core:window:allow-set-title",
    "core:window:allow-close",
    "core:window:allow-start-dragging",
    "core:app:default",
    "core:resources:default",
    "core:menu:default",
    "core:tray:default"
  ]
}
```

**Gotcha — frameless windows and drag regions.** When you set `"decorations": false` and rely on the `data-tauri-drag-region` HTML attribute (or the equivalent `window.startDragging()` call) to let users drag the window by its titlebar, the capability must include `core:window:allow-start-dragging`. The `core:window:default` set does **not** cover it — without this permission, clicking and holding on the drag region logs `window.start_dragging not allowed` in the webview console and the window sits still.

### Plugin Permissions

```json
{
  "permissions": [
    "fs:default",
    "fs:allow-read-file",
    "fs:allow-write-file",
    "shell:allow-open",
    "dialog:allow-open",
    "dialog:allow-save",
    "http:default",
    "clipboard-manager:allow-read",
    "clipboard-manager:allow-write"
  ]
}
```

## Platform-Specific Capabilities

Target specific platforms using the `platforms` array.

### Desktop-Only Capability

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "desktop-capability",
  "windows": ["main"],
  "platforms": ["linux", "macOS", "windows"],
  "permissions": [
    "global-shortcut:allow-register",
    "global-shortcut:allow-unregister",
    "shell:allow-execute"
  ]
}
```

### Mobile-Only Capability

```json
{
  "$schema": "../gen/schemas/mobile-schema.json",
  "identifier": "mobile-capability",
  "windows": ["main"],
  "platforms": ["iOS", "android"],
  "permissions": [
    "nfc:allow-scan",
    "biometric:allow-authenticate",
    "barcode-scanner:allow-scan"
  ]
}
```

### Separate Files per Platform

Split platform variants into distinct files for cleaner organization:

`src-tauri/capabilities/desktop.json`:
```json
{
  "identifier": "desktop-features",
  "windows": ["main"],
  "platforms": ["linux", "macOS", "windows"],
  "permissions": ["global-shortcut:default", "shell:default"]
}
```

`src-tauri/capabilities/mobile.json`:
```json
{
  "identifier": "mobile-features",
  "windows": ["main"],
  "platforms": ["iOS", "android"],
  "permissions": ["haptics:default", "biometric:default"]
}
```

## Multi-Window Application Example

`src-tauri/capabilities/main.json`:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main-window",
  "description": "Full access for main application window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "fs:default",
    "shell:allow-open",
    "dialog:default",
    "http:default",
    "clipboard-manager:default"
  ]
}
```

`src-tauri/capabilities/settings.json`:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "settings-window",
  "description": "Limited access for settings window",
  "windows": ["settings"],
  "permissions": [
    "core:window:allow-close",
    "core:event:default",
    "fs:allow-read-file",
    "fs:allow-write-file"
  ]
}
```

`src-tauri/capabilities/preview.json`:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "preview-window",
  "description": "Read-only access for preview window",
  "windows": ["preview"],
  "permissions": [
    "core:window:default",
    "core:event:default",
    "fs:allow-read-file"
  ]
}
```

## Remote API Access

Allow remote URLs to access Tauri commands (use with caution):

```json
{
  "$schema": "../gen/schemas/remote-schema.json",
  "identifier": "remote-capability",
  "windows": ["main"],
  "remote": {
    "urls": ["https://*.example.com"]
  },
  "permissions": ["http:default"]
}
```

## Best Practices

1. **Least privilege**: Grant only the permissions each window actually needs
2. **Separate by window**: Create distinct capability files per window role
3. **Test boundaries**: Verify unpermitted APIs are correctly denied

## Schema Support

Reference generated schemas in capability files for IDE autocompletion:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json"
}
```

Available schemas after build: `desktop-schema.json`, `mobile-schema.json`, `remote-schema.json`.

## Troubleshooting

### Permission Denied Errors

Check that the capability includes the required permission and targets the correct window label.

### Capability Not Applied

Verify the capability file is in `src-tauri/capabilities/` or explicitly listed in `tauri.conf.json`.

### Window Label Mismatch

Window labels in capabilities must match the labels defined when creating windows in Rust code. Labels are case-sensitive.

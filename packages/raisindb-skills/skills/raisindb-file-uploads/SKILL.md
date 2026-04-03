---
name: raisindb-file-uploads
description: "Upload, store, and display files using the raisin:Asset system. Covers single/batch uploads, progress tracking, signed URLs, and thumbnails. Use when adding file handling to your app."
---

# File Uploads and the raisin:Asset System

## Workspace Setup

### Option A: Files in the same workspace (default, simpler)

Files live alongside content in one workspace. Add `raisin:Asset` and `raisin:Folder` to `workspace_patches` in `manifest.yaml`:

```yaml
workspace_patches:
  my-workspace:
    allowed_node_types:
      add:
        - raisin:Asset
        - raisin:Folder
```

Create a files folder in your content tree (e.g., `/my-workspace/files/`) as a `raisin:Folder` node, then upload files into it. Upload with `workspace: 'my-workspace'`.

### Option B: Separate media workspace (shared library, different ACL)

For larger apps, create a dedicated workspace for assets. This is useful when:
- Multiple content workspaces share the same media library
- You want different permissions for files vs content (e.g., editors can upload, viewers can only read)

Create `workspaces/media.yaml`:

```yaml
name: media
title: Media Library
description: Shared file storage

allowed_node_types:
  - raisin:Asset
  - raisin:Folder

allowed_root_node_types:
  - raisin:Folder

root_structure:
  - name: files
    node_type: raisin:Folder
    title: Files
```

Register in `manifest.yaml`:

```yaml
provides:
  workspaces:
    - my-workspace
    - media
```

Upload to the media workspace:

```typescript
const batch = await client.uploadFiles(files, {
  repository: REPOSITORY,
  workspace: 'media',       // separate workspace
  basePath: '/media/files',
  concurrency: 3,
});
```

### Cross-workspace references

When content in one workspace references a file in another, store the asset path as a property and query the media workspace separately:

```typescript
// Content node has: properties.cover_image_path = '/media/files/hero.jpg'
const page = await queryOne(`SELECT * FROM content WHERE path = $1`, [pagePath]);

// Sign the URL from the media workspace
const mediaWs = db.workspace('media');
const { url } = await mediaWs.signAssetUrl(
  page.properties.cover_image_path, 'display'
);
```

### When to use which

| Pattern | Use when |
|---------|----------|
| Same workspace | Simple apps, files belong to specific content, one set of permissions |
| Separate workspace | Shared media library, multiple content workspaces, different ACL for files vs content |

## The Asset Model

Every uploaded file becomes a `raisin:Asset` node. The upload only creates the `file` Resource property. All other properties (`thumbnail`, `file_type`, `title`, etc.) are **empty by default** — you populate them by writing a server-side function triggered on upload. See "Asset Processing Pipeline" below.

| Property | Type | Set by |
|----------|------|--------|
| `file` | Resource | Upload API (automatic) |
| `thumbnail` | Resource | Your process-asset function (you build this) |
| `title` | String | Your process-asset function or user input |
| `file_type` | String | Your process-asset function (from MIME type) |
| `file_size` | Number | Your process-asset function |
| `description` | String | Your process-asset function or user input |
| `alt_text` | String | Your process-asset function (AI or user input) |
| `keywords` | Array | Your process-asset function (AI or user input) |

A `Resource` property looks like:

```json
{
  "uuid": "file-uuid",
  "name": "photo.jpeg",
  "size": 102400,
  "mime_type": "image/jpeg",
  "url": "storage-key-path"
}
```

You never construct this manually -- the upload API creates it.

## Single File Upload

```typescript
const client = new RaisinClient('ws://localhost:8080/sys/default/my-repo', { ... });

const file = document.querySelector('input[type="file"]').files[0];

const upload = await client.upload(file, {
  repository: 'my-repo',
  workspace: 'content',
  path: '/files/my-image.jpg',
  onProgress: (p) => {
    console.log(`${Math.round(p.progress * 100)}% - ${p.status}`);
    // p.speed (bytes/sec), p.eta (seconds), p.bytesUploaded, p.bytesTotal
  },
});

const result = await upload.start();
```

Upload controls: `upload.pause()`, `await upload.resume()`, `await upload.cancel()`.

## Batch Upload

Upload multiple files with concurrency control:

```typescript
const files = document.querySelector('input[type="file"]').files;

const batch = await client.uploadFiles(files, {
  repository: 'my-repo',
  workspace: 'content',
  basePath: '/files/uploads',
  concurrency: 3,
});

const result = await batch.start();
console.log('Successful:', result.successful.length);
console.log('Failed:', result.failed.length);
```

Batch also supports `pause()`, `resume()`, and `cancel()`.

## Progress Tracking

Add `onProgress`, `onFileComplete`, and `onFileError` callbacks to any upload call:

```typescript
const batch = await client.uploadFiles(files, {
  repository: 'my-repo',
  workspace: 'content',
  basePath: '/files/uploads',
  concurrency: 3,
  onProgress: (progress) => {
    console.log(`${progress.filesCompleted}/${progress.filesTotal} files, ${Math.round(progress.progress * 100)}%`);
    progress.files.forEach((f) => console.log(`  ${f.file}: ${Math.round(f.progress * 100)}% [${f.status}]`));
  },
  onFileComplete: (filename) => console.log('Uploaded:', filename),
  onFileError: (filename, error) => console.error('Failed:', filename, error.message),
});
await batch.start();
```

## Signed URLs (Always Required)

All binary file access goes through **signed URLs** -- time-limited, HMAC-signed URLs generated by the server (default 5 minutes). This applies to both anonymous and authenticated users. There is no direct/public URL bypass. The server validates read permission at signing time, then issues a URL anyone can use until it expires.

```typescript
const db = client.database('my-repo');
const ws = db.workspace('content');

// Display URL (renders inline)
const { url } = await ws.signAssetUrl('/files/my-image.jpg', 'display');

// Thumbnail URL
const { url: thumbUrl } = await ws.signAssetUrl('/files/my-image.jpg', 'display', {
  propertyPath: 'thumbnail',
});
```

Use in HTML:

```html
<img src={url} alt="My image" />
<img src={thumbUrl} alt="Thumbnail" />
```

## Download URLs

Force a download (Content-Disposition: attachment):

```typescript
const { url } = await ws.signAssetUrl('/files/document.pdf', 'download');
```

```html
<a href={url} download>Download PDF</a>
```

## Asset Processing Pipeline

RaisinDB supports **server-side functions** that run JavaScript on the server, triggered by events. This is how you add post-upload processing. The pattern:

1. **Trigger** watches for `raisin:Asset` node creation events
2. **Function** runs server-side JavaScript with access to the `raisin.*` runtime API
3. The function can: read the uploaded file, detect its MIME type, resize images, extract PDF text, generate thumbnails, call AI models, and update node properties

**Nothing happens automatically after upload** — the `raisin:Asset` node only has the `file` Resource. You build the processing logic as a trigger + function in your RAP package.

**BEFORE writing function code**: Run `npm install` in the project root (installs `@raisindb/functions-types`), then read `node_modules/@raisindb/functions-types/raisin.d.ts` — it is the complete API reference. Only use methods defined there.

### Built-in Server-Side Capabilities

The function runtime includes image processing and PDF handling — no external services needed:

| Capability | API | What it does |
|-----------|-----|-------------|
| **Read file metadata** | `node.getResource('./file')` | Returns Resource with `.mimeType`, `.size`, `.name` |
| **Resize images** | `resource.resize({ maxWidth, format, quality })` | Server-side image resize, returns thumbnail data |
| **Process PDFs** | `resource.processDocument({ ocr, generateThumbnail })` | Extract text, generate page thumbnail |
| **Store thumbnails** | `node.addResource('./thumbnail', data)` | Persist a Resource on any property path |
| **AI metadata** | `raisin.ai.completion({ model, messages })` | Call AI models for content analysis |
| **Update properties** | `raisin.sql.query(sql, params)` | Update node properties via JSONB merge |

See `raisindb-functions-triggers` skill for TypeScript types and the full Node Resource API reference.

**There is NO automatic thumbnail generation.** No built-in "AssetProcessing job" runs. If you want thumbnails, you MUST create a trigger + function that calls `resource.resize()`. The code below is the complete, working implementation.

### 1. Trigger: fire when asset upload completes

Create `content/functions/triggers/on-asset-ready/.node.yaml`:

```yaml
node_type: raisin:Trigger
properties:
  title: Process Uploaded Asset
  name: on-asset-ready
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
  filters:
    workspaces:
      - my-workspace
    node_types:
      - raisin:Asset
    property_filters:
      "file.metadata.storage_key":
        $exists: true
  priority: 10
  max_retries: 3
  function_path: /lib/myapp/process-asset
```

### 2. Function: detect file type and create thumbnail

**COPY THIS CODE EXACTLY.** Do not rewrite it. Do not simplify it. Do not "improve" it. Do not remove `node.getResource()` or `resource.resize()` calls. This code is verified and working.

The function runtime is NOT Node.js. See `raisindb-functions-triggers` skill for the TypeScript interface of `Node`, `Resource`, and all available methods. The key methods used below:

- `node.getResource('./file')` → returns `Resource` with `.mimeType`, `.resize()`, `.processDocument()`
- `resource.resize({ maxWidth, format, quality })` → resizes the image server-side, returns data
- `node.addResource('./thumbnail', data)` → stores the resized image as `properties.thumbnail`

Create `content/functions/lib/myapp/process-asset/.node.yaml`:

```yaml
node_type: raisin:Function
properties:
  name: process-asset
  title: Process Asset
  description: Detect file type and generate thumbnail for uploaded assets
  language: javascript
  entry_file: index.js:handler
  execution_mode: async
  enabled: true
```

Create `content/functions/lib/myapp/process-asset/index.js`:

```javascript
async function handler(context) {
  const { event, workspace } = context.flow_input;
  const node = await raisin.nodes.get(workspace, event.node_path);
  if (!node) return { success: false, error: 'Node not found' };

  const resource = node.getResource('./file');
  if (!resource) return { success: false, error: 'No file resource' };

  const isImage = resource.mimeType?.startsWith('image/');
  const isPdf = resource.mimeType === 'application/pdf';

  // Detect file type category
  let fileType = 'document';
  if (isImage) fileType = 'image';
  else if (resource.mimeType?.startsWith('video/')) fileType = 'video';
  else if (isPdf) fileType = 'pdf';

  // Generate thumbnail for images
  if (isImage) {
    const thumbnail = await resource.resize({
      maxWidth: 200,
      format: 'jpeg',
      quality: 80,
    });
    await node.addResource('./thumbnail', thumbnail);
  }

  // Generate thumbnail for PDFs
  if (isPdf) {
    const result = await resource.processDocument({
      generateThumbnail: true,
      thumbnailWidth: 200,
    });
    if (result.thumbnail) {
      await node.addResource('./thumbnail', result.thumbnail);
    }
  }

  // Update properties with file type metadata
  await raisin.sql.query(
    `UPDATE ${workspace} SET properties = properties || $1::jsonb WHERE path = $2`,
    [JSON.stringify({ file_type: fileType, content_type: fileType }), event.node_path]
  );

  return { success: true, file_type: fileType };
}

module.exports = { handler };
```

Register both in `manifest.yaml`:

```yaml
provides:
  functions:
    - /lib/myapp/process-asset
  triggers:
    - /triggers/on-asset-ready
```

See `raisindb-functions-triggers` skill for full function/trigger reference.

### 3. Frontend: display thumbnails with real-time updates

Thumbnails are generated asynchronously by the server-side function. The frontend renders immediately with a placeholder, then updates when the thumbnail appears via WebSocket events.

**DO NOT use `setTimeout` to wait for thumbnails. Subscribe to events instead.**

```typescript
// Load folder items
async function loadFiles() {
  items = await query(`
    SELECT * FROM content
    WHERE CHILD_OF($1) AND (node_type = 'raisin:Folder' OR node_type = 'raisin:Asset')
  `, [folderPath]);

  // Sign thumbnail URLs for items that already have them
  for (const item of items) {
    if (item.node_type === 'raisin:Asset' && item.properties.thumbnail) {
      const { url } = await ws.signAssetUrl(item.path, 'display', {
        propertyPath: 'thumbnail',
      });
      item._thumbnailUrl = url;
    }
  }
}

// Subscribe to changes — thumbnails appear via node:updated events
const db = client.database(REPOSITORY);
const workspace = db.workspace(WORKSPACE_NAME);
const subscription = await workspace.events().subscribe(
  {
    workspace: WORKSPACE_NAME,
    path: folderPath + '/**',
    event_types: ['node:created', 'node:updated', 'node:deleted'],
  },
  async () => {
    await loadFiles();  // re-fetch — thumbnails now available
  }
);

// Clean up
onDestroy(() => subscription.unsubscribe());
```

Render with placeholder for items where thumbnail is still processing:

```svelte
{#if item._thumbnailUrl}
  <img src={item._thumbnailUrl} alt={item.properties.title || item.name} />
{:else if item.node_type === 'raisin:Asset'}
  <!-- Placeholder while thumbnail is being generated server-side -->
  <div class="skeleton" />
{:else}
  <!-- Folder icon -->
{/if}
```

The flow: upload completes → asset node created → trigger fires → function generates thumbnail → `node:updated` event → subscription callback → `loadFiles()` re-fetches → thumbnail URL signed and rendered.

## Querying Assets

Use SQL to list, search, and filter assets:

```sql
-- List all assets in a folder
SELECT id, path, properties->>'title'::String AS title,
       properties->>'file_type'::String AS type,
       properties->>'file_size'::String AS size
FROM 'content'
WHERE node_type = 'raisin:Asset'
  AND CHILD_OF('/content/images')

-- Search assets by keyword
SELECT * FROM 'content'
WHERE node_type = 'raisin:Asset'
  AND FULLTEXT_MATCH('landscape photo', 'english')

-- Filter by MIME category
SELECT * FROM 'content'
WHERE node_type = 'raisin:Asset'
  AND properties->>'file_type'::String = 'image'
```

## File Browser Pattern

A complete file browser combines uploads, folder navigation, and drag-and-drop. Key pattern from the Launchpad example:

- **Query folder contents** -- `SELECT ... WHERE CHILD_OF($1) AND (node_type = 'raisin:Folder' OR node_type = 'raisin:Asset')`.
- **Upload to folder** -- pass the folder path as `basePath` to `uploadFiles()`.
- **Create folders** -- `INSERT INTO ws (path, node_type, properties) VALUES ($1, 'raisin:Folder', $2::jsonb)`.
- **Drag-and-drop** -- accept files via a drop zone component, call `uploadFiles()` with the drop target path.
- **Drop onto subfolders** -- detect folder drop targets, upload to that folder's path instead of the current one.
- **Real-time refresh** -- subscribe to `node:created`, `node:updated`, `node:deleted` events on the folder subtree and reload on changes.

```typescript
// Upload files to a folder
const batch = await client.uploadFiles(files, {
  repository: 'my-repo', workspace: 'content',
  basePath: targetFolderPath, concurrency: 3,
  onProgress: (p) => { /* update UI */ },
});
await batch.start();

// Create a subfolder
await query(`INSERT INTO content (path, node_type, properties)
  VALUES ($1, 'raisin:Folder', $2::jsonb)`,
  [`${parentPath}/${name}`, JSON.stringify({ description: '' })]);

// Load folder contents
const items = await query(`SELECT id, name, path, node_type, properties
  FROM content WHERE CHILD_OF($1)
  AND (node_type = 'raisin:Folder' OR node_type = 'raisin:Asset')`,
  [currentFolderPath]);
```

## MediaField in Archetypes

To add a file upload field to a content type, use `MediaField` in the archetype definition:

```yaml
# archetypes/blog-post.yaml
name: myapp:BlogPost
title: Blog Post
base_node_type: myapp:BlogPost

fields:
  - $type: TextField
    name: title
    title: Title
    required: true

  - $type: MediaField
    name: cover_image
    title: Cover Image

  - $type: SectionField
    name: content
    title: Content
```

This stores a `Resource` in the `cover_image` property. Display it in the frontend:

```typescript
const page = await queryOne(`
  SELECT path, properties FROM content WHERE path = $1
`, [pagePath]);

if (page.properties.cover_image?.url) {
  const { url } = await ws.signAssetUrl(page.path, 'display', {
    propertyPath: 'cover_image',
  });
  // Use url in an <img> tag
}
```

## Quick Reference

| Task | Method |
|------|--------|
| Upload one file | `client.upload(file, { repository, workspace, path })` |
| Upload many files | `client.uploadFiles(files, { repository, workspace, basePath, concurrency })` |
| Display URL | `ws.signAssetUrl(path, 'display')` |
| Download URL | `ws.signAssetUrl(path, 'download')` |
| Thumbnail URL | `ws.signAssetUrl(path, 'display', { propertyPath: 'thumbnail' })` |
| List folder assets | `SELECT ... WHERE CHILD_OF($1) AND node_type = 'raisin:Asset'` |
| Create folder | `INSERT INTO ws (path, node_type, ...) VALUES ($1, 'raisin:Folder', ...)` |
| Add upload field | `$type: MediaField` in archetype YAML |

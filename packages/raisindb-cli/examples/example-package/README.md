# Example Package Structure

This is an example of how to structure a RaisinDB package.

## Structure

```
example-package/
├── manifest.yaml        # Package manifest
├── nodetypes/           # Node type definitions (optional)
├── workspaces/          # Workspace definitions (optional)
├── content/             # Content nodes (optional)
├── functions/           # Function definitions (optional)
└── README.md            # Package documentation
```

## Creating a Package

1. Create a folder with your package contents
2. Add a `manifest.yaml` manifest with name, version, etc.
3. Run: `raisindb package create ./example-package`
4. Upload: `raisindb package upload example-package-1.0.0.rap`

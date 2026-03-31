# Pocket Medico RaisinDB Package

This package provides the node types, workspaces, and functions for the Pocket Medico medical transcription platform.

## Overview

Pocket Medico is a hybrid AI + human-powered medical transcription service for doctors. It allows doctors to:

1. Upload audio recordings or handwritten notes
2. Get AI-powered transcription (Light option)
3. Get human-verified transcription (Pro option)
4. Download professional medical documents

## Package Structure

```
pocketmedico/
├── manifest.yaml       # Package manifest
├── nodetypes/          # Custom node type definitions
├── workspaces/         # Workspace definitions
├── content/            # Default content (triggers, functions)
└── static/             # Static assets
```

## User Roles

- **Customer (Doctor)**: Creates transcription orders, uploads audio/notes, downloads transcripts
- **Nurse (Transcriber)**: Reviews and edits AI-generated transcriptions

## Installation

This package is installed as part of the pocketmedico example.

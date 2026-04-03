import type { TemplateVars, FileEntry, Pack } from '../types.js';
import { renderTemplate } from '../render.js';
import { manifest } from '../content/manifest.js';
import { workspace } from '../content/workspace.js';

function r(template: string, vars: TemplateVars): string {
  return renderTemplate(template, vars);
}

function rootFolderNode(vars: TemplateVars): string {
  return `node_type: raisin:Folder
properties:
  title: ${vars.packageName}
  description: ${vars.description}
`;
}

function rootPackageJson(vars: TemplateVars): string {
  return `{
  "name": "${vars.packageName}",
  "private": true,
  "scripts": {
    "validate": "raisindb package create ./package --check",
    "build": "raisindb package create ./package",
    "deploy": "raisindb package deploy ./package",
    "sync": "cd package && raisindb package sync . --watch",
    "dev": "cd frontend && npm run dev"
  },
  "devDependencies": {
    "@raisindb/functions-types": "^0.1.0"
  }
}
`;
}

function gitignore(): string {
  return `node_modules/
*.rap
.DS_Store
`;
}

function agentMd(vars: TemplateVars): string {
  return `# {{packageName}} — RaisinDB Application

{{description}}

## Setup

    npm install                                           # Install function types
    npx skills add maravilla-labs/raisindb/packages/raisindb-skills  # Install AI coding skills

## Project Structure

    package/     — RaisinDB content package (YAML schemas, content, functions)
    frontend/    — Web frontend (SvelteKit or React)

## Commands

    npm run validate    # Validate package YAML
    npm run deploy      # Build and upload package to server
    npm run sync        # Live sync package changes during dev
    npm run dev         # Start frontend dev server
`;
}

function rootReadme(vars: TemplateVars): string {
  return `# {{packageName}}

{{description}}

## Getting Started

1. Install dependencies and AI coding skills:

       npm install
       npx skills add maravilla-labs/raisindb/packages/raisindb-skills

2. Define your content model in \`package/nodetypes/\`, \`package/archetypes/\`, \`package/elementtypes/\`

3. Add initial content in \`package/content/{{workspace}}/\`

4. Validate your package:

       npm run validate

5. Deploy to server:

       npm run deploy

6. Set up your frontend in \`frontend/\` (SvelteKit or React)

## Development

    npm run sync    # Live sync package changes
    npm run dev     # Start frontend dev server
`;
}

function frontendReadme(vars: TemplateVars): string {
  return `# {{packageName}} Frontend

Set up your frontend here using SvelteKit or React.

Install AI coding skills for step-by-step guidance:

    npx skills add maravilla-labs/raisindb/packages/raisindb-skills

Then ask your AI agent to scaffold the frontend using the
\`raisindb-frontend-sveltekit\` or \`raisindb-frontend-react\` skill.
`;
}

/** Prefix all paths in an array with a subdirectory */
function prefixed(prefix: string, files: FileEntry[]): FileEntry[] {
  return files.map(f => ({ path: `${prefix}/${f.path}`, content: f.content }));
}

export const minimalPack: Pack = {
  name: 'minimal',
  description: 'Minimal package with AI agent skills support',
  getFiles(vars: TemplateVars): FileEntry[] {
    const packageFiles: FileEntry[] = [
      { path: 'manifest.yaml', content: r(manifest(vars), vars) },

      // Empty directories
      { path: 'nodetypes/.gitkeep', content: '' },
      { path: 'mixins/.gitkeep', content: '' },
      { path: 'archetypes/.gitkeep', content: '' },
      { path: 'elementtypes/.gitkeep', content: '' },
      { path: 'static/.gitkeep', content: '' },
      { path: `content/functions/lib/${vars.namespace}/.gitkeep`, content: '' },
      { path: 'content/functions/triggers/.gitkeep', content: '' },

      // Workspace
      { path: `workspaces/${vars.workspace}.yaml`, content: r(workspace(vars), vars) },

      // Root content folder
      { path: `content/${vars.workspace}/${vars.workspace}/.node.yaml`, content: rootFolderNode(vars) },
    ];

    return [
      // Root-level files
      { path: 'package.json', content: rootPackageJson(vars) },
      { path: '.gitignore', content: gitignore() },
      { path: 'AGENT.md', content: r(agentMd(vars), vars) },
      { path: 'README.md', content: r(rootReadme(vars), vars) },

      // Package files
      ...prefixed('package', packageFiles),

      // Frontend placeholder
      { path: 'frontend/README.md', content: r(frontendReadme(vars), vars) },
    ];
  },
};

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

function agentMd(vars: TemplateVars): string {
  return `# {{packageName}} — RaisinDB Application

{{description}}

## AI Coding Skills

This project uses RaisinDB. Install agent skills for full AI coding assistance:

    npx skills add raisindb/raisindb/packages/raisindb-skills

Skills teach your AI agent how to build both the content package and the frontend.

## Project Structure

    package/     — RaisinDB content package (YAML schemas, content, functions)
    frontend/    — Web frontend (SvelteKit or React)

## Quick Reference

    cd package && raisindb package create --check .   # Validate package
    cd package && raisindb package create .            # Build .rap file
    raisindb package upload {{packageName}}-0.1.0.rap  # Upload to server
    cd package && raisindb package sync . --watch      # Live sync during dev
`;
}

function rootReadme(vars: TemplateVars): string {
  return `# {{packageName}}

{{description}}

## Getting Started

1. Install AI coding skills (recommended):

       npx skills add raisindb/raisindb/packages/raisindb-skills

2. Define your content model in \`package/nodetypes/\`, \`package/archetypes/\`, \`package/elementtypes/\`

3. Add initial content in \`package/content/{{workspace}}/\`

4. Validate your package:

       cd package && raisindb package create --check .

5. Build and deploy:

       cd package && raisindb package create .
       raisindb package upload {{packageName}}-0.1.0.rap

6. Set up your frontend in \`frontend/\` (SvelteKit or React)

## Development

    cd package && raisindb package sync . --watch   # Live sync package changes
    cd frontend && npm run dev                      # Start frontend dev server
`;
}

function frontendReadme(vars: TemplateVars): string {
  return `# {{packageName}} Frontend

Set up your frontend here using SvelteKit or React.

Install AI coding skills for step-by-step guidance:

    npx skills add raisindb/raisindb/packages/raisindb-skills

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
      { path: 'AGENT.md', content: r(agentMd(vars), vars) },
      { path: 'README.md', content: r(rootReadme(vars), vars) },

      // Package files
      ...prefixed('package', packageFiles),

      // Frontend placeholder
      { path: 'frontend/README.md', content: r(frontendReadme(vars), vars) },
    ];
  },
};

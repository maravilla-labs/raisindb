import type { TemplateVars, FileEntry, Pack } from '../types.js';
import { renderTemplate } from '../render.js';
import { manifest } from '../content/manifest.js';
import { rootReadme, packageReadme, frontendReadme } from '../content/readme.js';
import { workspace } from '../content/workspace.js';
import { agentMd, claudeMd, geminiMd } from '../content/entry-points.js';
import { productContext, architectureContext, decisionsContext } from '../content/context.js';
import { principlesMd, repoMapMd, workflowsMd } from '../content/agents-shared.js';
import { createNodeTypeTask } from '../content/agents-tasks.js';
import { schemasMd, nodeTypePromptMd } from '../content/domain.js';
import { sqlKnowledge } from '../content/knowledge/sql.js';
import { nodeTypesKnowledge } from '../content/knowledge/node-types.js';
import { triggersKnowledge } from '../content/knowledge/triggers.js';
import { flowsKnowledge } from '../content/knowledge/flows.js';
import { functionsOverviewKnowledge } from '../content/knowledge/functions/overview.js';
import { functionsJavascriptKnowledge } from '../content/knowledge/functions/javascript.js';
import { functionsStarlarkKnowledge } from '../content/knowledge/functions/starlark.js';
import { sdkOverviewKnowledge } from '../content/knowledge/sdk/overview.js';
import { sdkNodesKnowledge } from '../content/knowledge/sdk/nodes.js';
import { sdkEventsKnowledge } from '../content/knowledge/sdk/events.js';
import { sdkSqlKnowledge } from '../content/knowledge/sdk/sql.js';
import { sdkFlowsAndChatKnowledge } from '../content/knowledge/sdk/flows-and-chat.js';
import { sdkAssetsKnowledge } from '../content/knowledge/sdk/assets.js';

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

/** Prefix all paths in an array with a subdirectory */
function prefixed(prefix: string, files: FileEntry[]): FileEntry[] {
  return files.map(f => ({ path: `${prefix}/${f.path}`, content: f.content }));
}

export const contentModelingPack: Pack = {
  name: 'content-modeling',
  description: 'Content modeling package with agent coding instructions',
  getFiles(vars: TemplateVars): FileEntry[] {
    // RaisinDB package files (go inside package/ subdirectory)
    const packageFiles: FileEntry[] = [
      { path: 'manifest.yaml', content: r(manifest(vars), vars) },
      { path: 'README.md', content: r(packageReadme(vars), vars) },

      // Empty directories with .gitkeep
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
      { path: 'README.md', content: r(rootReadme(vars), vars) },
      { path: 'AGENT.md', content: r(agentMd(vars), vars) },
      { path: 'CLAUDE.md', content: r(claudeMd(vars), vars) },
      { path: 'GEMINI.md', content: r(geminiMd(vars), vars) },

      // Agent team config (root level — covers whole project)
      { path: 'AGENTS/shared/principles.md', content: principlesMd() },
      { path: 'AGENTS/shared/repo-map.md', content: repoMapMd() },
      { path: 'AGENTS/shared/workflows.md', content: workflowsMd() },
      { path: 'AGENTS/tasks/create-node-type.md', content: createNodeTypeTask() },

      // Agent knowledge base (root level — covers whole project)
      { path: '.agent/context/product.md', content: r(productContext(vars), vars) },
      { path: '.agent/context/architecture.md', content: architectureContext() },
      { path: '.agent/context/decisions.md', content: decisionsContext() },
      { path: '.agent/domain/schemas.md', content: schemasMd() },
      { path: '.agent/prompts/node-types.md', content: nodeTypePromptMd() },
      { path: '.agent/knowledge/sql.md', content: sqlKnowledge() },
      { path: '.agent/knowledge/node-types.md', content: nodeTypesKnowledge() },
      { path: '.agent/knowledge/triggers.md', content: triggersKnowledge() },
      { path: '.agent/knowledge/flows.md', content: flowsKnowledge() },
      { path: '.agent/knowledge/functions/overview.md', content: functionsOverviewKnowledge() },
      { path: '.agent/knowledge/functions/javascript.md', content: functionsJavascriptKnowledge() },
      { path: '.agent/knowledge/functions/starlark.md', content: functionsStarlarkKnowledge() },
      { path: '.agent/knowledge/sdk/overview.md', content: sdkOverviewKnowledge() },
      { path: '.agent/knowledge/sdk/nodes.md', content: sdkNodesKnowledge() },
      { path: '.agent/knowledge/sdk/events.md', content: sdkEventsKnowledge() },
      { path: '.agent/knowledge/sdk/sql.md', content: sdkSqlKnowledge() },
      { path: '.agent/knowledge/sdk/flows-and-chat.md', content: sdkFlowsAndChatKnowledge() },
      { path: '.agent/knowledge/sdk/assets.md', content: sdkAssetsKnowledge() },

      // Package files under package/
      ...prefixed('package', packageFiles),

      // Frontend placeholder
      { path: 'frontend/README.md', content: r(frontendReadme(vars), vars) },
    ];
  },
};

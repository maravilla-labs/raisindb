import type { TemplateVars } from '../types.js';

export function workspace(vars: TemplateVars): string {
  return `name: {{workspace}}
title: {{workspace}}
description: {{description}}
icon: layout-grid
color: "#6366f1"

allowed_node_types:
  - raisin:Folder

allowed_root_node_types:
  - raisin:Folder

root_structure:
  - name: content
    node_type: raisin:Folder
    title: Content
    description: Root content folder
`;
}

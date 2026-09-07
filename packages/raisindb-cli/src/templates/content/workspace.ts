import type { TemplateVars } from '../types.js';

// Keys mirror `raisin_models::workspace::Workspace`, which is what the package
// installer deserializes this file into; any other key is dropped silently.
export function workspace(vars: TemplateVars): string {
  return `name: {{workspace}}
description: {{description}}

allowed_node_types:
  - raisin:Folder

allowed_root_node_types:
  - raisin:Folder

# Root-level nodes created when the workspace is first installed.
initial_structure:
  children:
    - name: content
      node_type: raisin:Folder
      properties:
        description: Root content folder
`;
}

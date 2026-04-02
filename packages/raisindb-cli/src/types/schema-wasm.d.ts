declare module '@raisindb/schema-wasm' {
  export function init(): void;
  export function validate_nodetype_yaml(yaml: string): string;
  export function validate_workspace_yaml(yaml: string): string;
  export function validate_manifest_yaml(yaml: string): string;
}

export interface TemplateVars {
  packageName: string;
  workspace: string;
  description: string;
  namespace: string;
}

export interface FileEntry {
  path: string;
  content: string;
}

export interface Pack {
  name: string;
  description: string;
  getFiles(vars: TemplateVars): FileEntry[];
}

import fs from 'fs';
import path from 'path';
import type { TemplateVars, FileEntry } from './types.js';

export function renderTemplate(template: string, vars: TemplateVars): string {
  return template
    .replace(/\{\{packageName\}\}/g, vars.packageName)
    .replace(/\{\{workspace\}\}/g, vars.workspace)
    .replace(/\{\{description\}\}/g, vars.description)
    .replace(/\{\{namespace\}\}/g, vars.namespace);
}

export function writeFileTree(baseDir: string, files: FileEntry[]): number {
  let count = 0;
  for (const file of files) {
    const fullPath = path.join(baseDir, file.path);
    const dir = path.dirname(fullPath);
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(fullPath, file.content, 'utf-8');
    count++;
  }
  return count;
}

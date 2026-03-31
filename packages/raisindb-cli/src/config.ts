import fs from 'fs';
import path from 'path';
import os from 'os';
import yaml from 'yaml';

export interface Config {
  server: string | null;
  token: string | null;
  default_repo: string | null;
}

const CONFIG_FILENAME = '.raisinrc';

/**
 * Searches up the directory tree for .raisinrc config file
 */
function findConfigFile(): string | null {
  let currentDir = process.cwd();
  const root = path.parse(currentDir).root;

  while (currentDir !== root) {
    const configPath = path.join(currentDir, CONFIG_FILENAME);
    if (fs.existsSync(configPath)) {
      return configPath;
    }
    currentDir = path.dirname(currentDir);
  }

  return null;
}

/**
 * Gets the config file path (searching up tree, then falling back to home directory)
 */
function getConfigPath(): string {
  // First try to find in current directory tree
  const foundConfig = findConfigFile();
  if (foundConfig) {
    return foundConfig;
  }

  // Fall back to home directory
  return path.join(os.homedir(), CONFIG_FILENAME);
}

/**
 * Loads the configuration from .raisinrc file
 */
export function loadConfig(): Config {
  const configPath = getConfigPath();

  if (!fs.existsSync(configPath)) {
    return {
      server: null,
      token: null,
      default_repo: null,
    };
  }

  try {
    const content = fs.readFileSync(configPath, 'utf-8');
    const config = yaml.parse(content);

    return {
      server: config.server || null,
      token: config.token || null,
      default_repo: config.default_repo || null,
    };
  } catch (error) {
    console.error(`Error reading config file: ${error instanceof Error ? error.message : String(error)}`);
    return {
      server: null,
      token: null,
      default_repo: null,
    };
  }
}

/**
 * Saves the configuration to .raisinrc file
 */
export function saveConfig(config: Config): void {
  const configPath = getConfigPath();

  try {
    const content = yaml.stringify({
      server: config.server,
      token: config.token,
      default_repo: config.default_repo,
    });

    fs.writeFileSync(configPath, content, 'utf-8');
  } catch (error) {
    console.error(`Error writing config file: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/**
 * Gets the current server URL
 */
export function getServer(): string | null {
  const config = loadConfig();
  return config.server;
}

/**
 * Sets the server URL
 */
export function setServer(server: string): void {
  const config = loadConfig();
  config.server = server;
  saveConfig(config);
}

/**
 * Gets the default repository/database
 */
export function getDefaultRepo(): string | null {
  const config = loadConfig();
  return config.default_repo;
}

/**
 * Sets the default repository/database
 */
export function setDefaultRepo(repo: string): void {
  const config = loadConfig();
  config.default_repo = repo;
  saveConfig(config);
}

import React, { useState } from 'react';
import { Box, Text, useInput } from 'ink';
import TextInput from 'ink-text-input';
import yaml from 'yaml';
import fs from 'fs';
import path from 'path';
import HelpScreen from './HelpScreen.js';
import Spinner from './Spinner.js';
import { SuccessMessage, ErrorMessage, InfoMessage } from './StatusMessages.js';
import { listNodes, getNodeByPath, listWorkspaces, getNodeFull } from '../api.js';
import FileAutocomplete, { getFileSuggestions, findCommonPrefix } from './FileAutocomplete.js';

interface ShellProps {
  currentDatabase: string | null;
  onCommand: (command: string, args: string[]) => Promise<any>;
}

interface Message {
  type: 'success' | 'error' | 'info' | 'output';
  text: string;
}

interface NavigationState {
  workspace: string | null;  // Current workspace (null = at database root)
  currentPath: string[];  // Array of node names for the path within workspace
}

interface CompletionState {
  suggestions: string[];
  lastTabTime: number;
  lastInput: string;
}

const Shell: React.FC<ShellProps> = ({ currentDatabase, onCommand }) => {
  const [input, setInput] = useState('');
  const [messages, setMessages] = useState<Message[]>([]);
  const [showHelp, setShowHelp] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  // Command history
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedInput, setSavedInput] = useState('');

  // Navigation state - filesystem-like hierarchy: database > workspace > nodes
  const [nav, setNav] = useState<NavigationState>({
    workspace: null,
    currentPath: [],
  });

  // Tab completion state
  const [completion, setCompletion] = useState<CompletionState>({
    suggestions: [],
    lastTabTime: 0,
    lastInput: '',
  });

  // @ file autocomplete state
  const [fileAutocomplete, setFileAutocomplete] = useState({
    visible: false,
    query: '',
    selectedIndex: 0,
    atPosition: -1, // Position of @ in input
  });

  // Commands that support @ file autocomplete
  const fileAutocompleteCommands = ['/upload', '/install', '/create'];

  // Ctrl+C state for double-press to exit
  const [ctrlCPressed, setCtrlCPressed] = useState(false);

  // Get current path as string for SQL queries
  const getCurrentPathStr = () => {
    return nav.currentPath.length > 0 ? `/${nav.currentPath.join('/')}` : '/';
  };

  // Build prompt with path (filesystem-like)
  const buildPrompt = () => {
    let prompt = 'raisindb';
    if (currentDatabase) {
      prompt += `:${currentDatabase}`;
      if (nav.workspace) {
        const pathStr = nav.currentPath.length > 0 ? `/${nav.currentPath.join('/')}` : '';
        prompt += `/${nav.workspace}${pathStr}`;
      }
    }
    return `${prompt}> `;
  };

  const prompt = buildPrompt();

  // Parse path into directory and partial name
  const parsePath = (pathArg: string): { dirPath: string; partial: string; prefix: string } => {
    // Normalize ./ prefix
    const normalized = pathArg.replace(/^\.\//, './');

    const lastSlash = normalized.lastIndexOf('/');
    if (lastSlash === -1) {
      // No slash - complete from current directory
      return { dirPath: getCurrentPathStr(), partial: normalized, prefix: '' };
    }

    // Has slash - split into directory and partial
    const prefix = normalized.substring(0, lastSlash + 1); // Include trailing slash
    const partial = normalized.substring(lastSlash + 1);

    // Build full directory path
    let dirPath: string;
    if (normalized.startsWith('/')) {
      // Absolute path
      dirPath = normalized.substring(0, lastSlash) || '/';
    } else if (normalized.startsWith('./')) {
      // Explicit relative path
      const relPath = normalized.substring(2, lastSlash);
      dirPath = nav.currentPath.length > 0
        ? `/${nav.currentPath.join('/')}/${relPath}`.replace(/\/+/g, '/')
        : `/${relPath}`;
    } else {
      // Implicit relative path
      const relPath = normalized.substring(0, lastSlash);
      dirPath = nav.currentPath.length > 0
        ? `/${nav.currentPath.join('/')}/${relPath}`.replace(/\/+/g, '/')
        : `/${relPath}`;
    }

    return { dirPath, partial, prefix };
  };

  // Get filesystem completions for local paths (used by /create and /upload commands)
  const getFilesystemCompletions = (pathArg: string): { matches: string[]; prefix: string } => {
    try {
      // Handle empty or starting with ./
      let searchPath = pathArg || './';

      // Normalize the path
      const isRelative = !path.isAbsolute(searchPath);
      const basePath = isRelative ? process.cwd() : '';

      // Find the directory to search and the partial filename
      let dirToSearch: string;
      let partial: string;
      let prefix: string;

      const lastSlash = searchPath.lastIndexOf('/');
      if (lastSlash === -1) {
        // No slash - search current directory
        dirToSearch = basePath || '.';
        partial = searchPath;
        prefix = '';
      } else {
        // Has slash
        prefix = searchPath.substring(0, lastSlash + 1);
        partial = searchPath.substring(lastSlash + 1);
        const dirPart = searchPath.substring(0, lastSlash) || '.';
        dirToSearch = isRelative ? path.join(basePath, dirPart) : dirPart;
      }

      // Read directory
      if (!fs.existsSync(dirToSearch)) {
        return { matches: [], prefix };
      }

      const entries = fs.readdirSync(dirToSearch, { withFileTypes: true });
      const matches = entries
        .filter((entry: any) => !entry.name.startsWith('.')) // Skip hidden files
        .filter((entry: any) => entry.name.toLowerCase().startsWith(partial.toLowerCase()))
        .map((entry: any) => entry.isDirectory() ? `${entry.name}/` : entry.name);

      return { matches, prefix };
    } catch {
      return { matches: [], prefix: '' };
    }
  };

  // Get completions for current input
  const getCompletions = async (pathArg: string): Promise<{ matches: string[]; prefix: string }> => {
    if (!currentDatabase) return { matches: [], prefix: '' };

    try {
      // At database root - complete workspace names
      if (!nav.workspace) {
        const workspaces = await listWorkspaces(currentDatabase);
        const matches = workspaces
          .map(w => w.name)
          .filter(name => name.toLowerCase().startsWith(pathArg.toLowerCase()));
        return { matches, prefix: '' };
      }

      // Parse the path to get directory and partial
      const { dirPath, partial, prefix } = parsePath(pathArg);

      // Get nodes from the target directory
      const nodes = await listNodes(currentDatabase, nav.workspace, dirPath);
      const matches = nodes
        .map(n => n.has_children ? `${n.name}/` : n.name)
        .filter(name => name.toLowerCase().startsWith(partial.toLowerCase()));

      return { matches, prefix };
    } catch {
      return { matches: [], prefix: '' };
    }
  };

  // Find longest common prefix
  const findCommonPrefixStrings = (strings: string[]): string => {
    if (strings.length === 0) return '';
    if (strings.length === 1) return strings[0];

    let prefix = strings[0];
    for (let i = 1; i < strings.length; i++) {
      while (!strings[i].toLowerCase().startsWith(prefix.toLowerCase())) {
        prefix = prefix.slice(0, -1);
        if (prefix === '') return '';
      }
      // Use the casing from first match
      prefix = strings[0].slice(0, prefix.length);
    }
    return prefix;
  };

  // Handle input change and @ detection
  const handleInputChange = (newValue: string) => {
    setInput(newValue);

    // Check if we're in a command that supports @ autocomplete
    const parts = newValue.trim().split(/\s+/);
    const command = parts[0]?.toLowerCase() || '';

    if (!fileAutocompleteCommands.includes(command)) {
      // Not a supported command, hide autocomplete
      if (fileAutocomplete.visible) {
        setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
      }
      return;
    }

    // Find @ in the input (after the command)
    const commandEndIndex = newValue.indexOf(' ');
    if (commandEndIndex === -1) {
      // No space yet, just the command
      if (fileAutocomplete.visible) {
        setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
      }
      return;
    }

    const afterCommand = newValue.substring(commandEndIndex + 1);
    const atIndex = afterCommand.lastIndexOf('@');

    if (atIndex === -1) {
      // No @ found
      if (fileAutocomplete.visible) {
        setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
      }
      return;
    }

    // Found @, extract the query after it
    const query = afterCommand.substring(atIndex + 1);
    const absoluteAtPosition = commandEndIndex + 1 + atIndex;

    // Get suggestions to check if there are any
    const suggestions = getFileSuggestions(query);

    setFileAutocomplete({
      visible: suggestions.length > 0,
      query,
      selectedIndex: 0,
      atPosition: absoluteAtPosition,
    });
  };

  // Handle @ autocomplete selection
  const handleFileAutocompleteSelect = (selectedPath: string) => {
    if (fileAutocomplete.atPosition === -1) return;

    // Replace @query with the selected path
    const beforeAt = input.substring(0, fileAutocomplete.atPosition);
    const newInput = `${beforeAt}${selectedPath}`;

    setInput(newInput);
    setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
  };

  // Handle tab completion
  const handleTabCompletion = async () => {
    // If @ autocomplete is visible, handle it
    if (fileAutocomplete.visible) {
      const suggestions = getFileSuggestions(fileAutocomplete.query);
      if (suggestions.length === 1) {
        // Single match - select it
        handleFileAutocompleteSelect(suggestions[0].fullPath);
      } else if (suggestions.length > 1) {
        // Multiple matches - complete common prefix
        const commonPrefix = findCommonPrefix(suggestions);
        if (commonPrefix.length > fileAutocomplete.query.length) {
          // Update input with common prefix
          const beforeAt = input.substring(0, fileAutocomplete.atPosition);
          const newInput = `${beforeAt}${commonPrefix}`;
          setInput(newInput);
          setFileAutocomplete(prev => ({ ...prev, query: commonPrefix }));
        }
      }
      return;
    }

    // Parse input to find command and argument
    const parts = input.trim().split(/\s+/);
    const command = parts[0]?.toLowerCase() || '';
    const arg = parts.slice(1).join(' ') || '';

    // Commands that support database node completion
    const nodeCompletableCommands = ['cd', 'cat', 'ls'];
    // Commands that support filesystem completion
    const filesystemCompletableCommands = ['/create', '/upload'];

    const isNodeCommand = nodeCompletableCommands.includes(command);
    const isFilesystemCommand = filesystemCompletableCommands.includes(command);

    if (parts.length < 2 && !isNodeCommand && !isFilesystemCommand) {
      // No argument yet or not a completable command
      return;
    }

    // For node commands, require a database connection
    if (isNodeCommand && !currentDatabase) return;

    // Get partial path to complete
    const pathArg = parts.length >= 2 ? arg : '';

    const now = Date.now();
    const isDoubleTab = (now - completion.lastTabTime < 500) && (completion.lastInput === input);

    // Use appropriate completion function
    let matches: string[];
    let prefix: string;

    if (isFilesystemCommand) {
      // Use filesystem completion for /create command
      const result = getFilesystemCompletions(pathArg);
      matches = result.matches;
      prefix = result.prefix;
    } else {
      // Use database node completion for cd, cat, ls
      const result = await getCompletions(pathArg);
      matches = result.matches;
      prefix = result.prefix;
    }

    if (matches.length === 0) {
      // No matches
      return;
    }

    if (matches.length === 1) {
      // Single match - complete it with full path
      const completed = `${command} ${prefix}${matches[0]}`;
      setInput(completed);
      setCompletion({ suggestions: [], lastTabTime: now, lastInput: completed });
    } else if (isDoubleTab) {
      // Double tab - show all matches
      setMessages(prev => [...prev, { type: 'info', text: matches.join('  ') }]);
      setCompletion({ suggestions: matches, lastTabTime: now, lastInput: input });
    } else {
      // Single tab with multiple matches - complete common prefix
      const commonPrefix = findCommonPrefixStrings(matches);
      // Get the partial name (after last slash or full arg)
      const lastSlash = pathArg.lastIndexOf('/');
      const currentPartial = lastSlash >= 0 ? pathArg.substring(lastSlash + 1) : pathArg;

      if (commonPrefix.length > currentPartial.length) {
        const completed = `${command} ${prefix}${commonPrefix}`;
        setInput(completed);
        setCompletion({ suggestions: matches, lastTabTime: now, lastInput: completed });
      } else {
        // No more common prefix, just record the tab
        setCompletion({ suggestions: matches, lastTabTime: now, lastInput: input });
      }
    }
  };

  // Handle up/down arrows for history and tab for completion
  useInput((inputChar, key) => {
    if (showHelp || isLoading) return;

    // Handle Ctrl+C
    if (key.ctrl && inputChar === 'c') {
      if (ctrlCPressed) {
        // Second Ctrl+C - exit
        process.exit(0);
      } else {
        // First Ctrl+C - clear input and show hint
        setInput('');
        setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
        setCtrlCPressed(true);
        // Reset after 2 seconds
        setTimeout(() => setCtrlCPressed(false), 2000);
      }
      return;
    }

    // Any other key resets Ctrl+C state
    if (ctrlCPressed) {
      setCtrlCPressed(false);
    }

    // Handle Escape - close autocomplete dropdown
    if (key.escape) {
      if (fileAutocomplete.visible) {
        setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });
        return;
      }
    }

    if (key.tab) {
      handleTabCompletion();
      return;
    }

    // When file autocomplete is visible, arrow keys navigate the dropdown
    if (fileAutocomplete.visible) {
      const suggestions = getFileSuggestions(fileAutocomplete.query);
      const maxIndex = Math.min(suggestions.length - 1, 7); // Max 8 items (0-7)

      if (key.upArrow) {
        setFileAutocomplete(prev => ({
          ...prev,
          selectedIndex: prev.selectedIndex > 0 ? prev.selectedIndex - 1 : maxIndex,
        }));
        return;
      }

      if (key.downArrow) {
        setFileAutocomplete(prev => ({
          ...prev,
          selectedIndex: prev.selectedIndex < maxIndex ? prev.selectedIndex + 1 : 0,
        }));
        return;
      }

      // Enter selects the current item
      if (key.return) {
        if (suggestions.length > 0 && fileAutocomplete.selectedIndex < suggestions.length) {
          handleFileAutocompleteSelect(suggestions[fileAutocomplete.selectedIndex].fullPath);
        }
        return;
      }
    }

    // History navigation (only when autocomplete not visible)
    if (key.upArrow) {
      if (history.length === 0) return;

      if (historyIndex === -1) {
        // First time pressing up, save current input
        setSavedInput(input);
        setHistoryIndex(history.length - 1);
        setInput(history[history.length - 1]);
      } else if (historyIndex > 0) {
        setHistoryIndex(historyIndex - 1);
        setInput(history[historyIndex - 1]);
      }
    } else if (key.downArrow) {
      if (historyIndex === -1) return;

      if (historyIndex < history.length - 1) {
        setHistoryIndex(historyIndex + 1);
        setInput(history[historyIndex + 1]);
      } else {
        // Back to current input
        setHistoryIndex(-1);
        setInput(savedInput);
      }
    }
  });

  // Handle ls command - lists workspaces at db root, nodes inside workspace
  const handleLs = async (args: string[]): Promise<{ type: string; message: string }> => {
    if (!currentDatabase) {
      return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
    }

    const longFormat = args.includes('-l') || args.includes('-lh');
    // Get path argument (filter out flags)
    const pathArg = args.find(a => !a.startsWith('-'));

    try {
      // At database root - list workspaces
      if (!nav.workspace) {
        const workspaces = await listWorkspaces(currentDatabase);
        if (workspaces.length === 0) {
          return { type: 'info', message: '(no workspaces)' };
        }

        let output: string;
        if (longFormat) {
          const lines = workspaces.map(w => {
            const desc = w.description || '';
            return `d workspace       ${w.name}${desc ? `  ${desc}` : ''}`;
          });
          output = lines.join('\n');
        } else {
          output = workspaces.map(w => `${w.name}/`).join('  ');
        }
        return { type: 'info', message: output };
      }

      // Inside workspace - list nodes at specified or current path
      let targetPath: string;
      if (pathArg) {
        // Normalize path - strip ./ prefix
        let normalizedPath = pathArg.replace(/^\.\//, '');

        // Handle absolute vs relative path
        if (normalizedPath.startsWith('/')) {
          targetPath = normalizedPath;
        } else {
          // Relative path - append to current path
          targetPath = nav.currentPath.length > 0
            ? `/${nav.currentPath.join('/')}/${normalizedPath}`
            : `/${normalizedPath}`;
        }
        // Remove trailing slash for consistency
        targetPath = targetPath.replace(/\/+$/, '') || '/';
      } else {
        targetPath = getCurrentPathStr();
      }

      const nodes = await listNodes(currentDatabase, nav.workspace, targetPath);

      if (nodes.length === 0) {
        return { type: 'info', message: '(empty)' };
      }

      let output: string;
      if (longFormat) {
        // Long format: type, name
        const lines = nodes.map(n => {
          const typeIcon = n.has_children ? 'd' : '-';
          const typeStr = n.node_type.padEnd(15);
          return `${typeIcon} ${typeStr} ${n.name}`;
        });
        output = lines.join('\n');
      } else {
        // Short format: just names
        output = nodes.map(n => n.has_children ? `${n.name}/` : n.name).join('  ');
      }

      return { type: 'info', message: output };
    } catch (error) {
      return { type: 'error', message: `Failed to list: ${error instanceof Error ? error.message : String(error)}` };
    }
  };

  // Handle cd command - enters workspace at db root, navigates nodes inside workspace
  const handleCd = async (args: string[]): Promise<{ type: string; message: string } | null> => {
    if (!currentDatabase) {
      return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
    }

    const target = args[0] || '';

    // cd / - go to database root
    if (!target || target === '/') {
      setNav({ workspace: null, currentPath: [] });
      return null;
    }

    // cd .. - go up one level
    if (target === '..') {
      if (!nav.workspace) {
        // Already at database root
        return { type: 'info', message: 'Already at database root' };
      }

      if (nav.currentPath.length === 0) {
        // At workspace root, go back to database root
        setNav({ workspace: null, currentPath: [] });
        return null;
      }

      // Go up one level in node hierarchy
      const newPath = nav.currentPath.slice(0, -1);
      setNav(prev => ({ ...prev, currentPath: newPath }));
      return null;
    }

    // At database root - cd into a workspace
    if (!nav.workspace) {
      // Verify workspace exists
      try {
        const workspaces = await listWorkspaces(currentDatabase);
        const ws = workspaces.find(w => w.name === target);
        if (!ws) {
          return { type: 'error', message: `No such workspace: ${target}` };
        }
        setNav({ workspace: target, currentPath: [] });
        return null;
      } catch (error) {
        return { type: 'error', message: `Failed to access workspace: ${error instanceof Error ? error.message : String(error)}` };
      }
    }

    // Inside workspace - navigate to a node
    try {
      // Normalize path - strip ./ prefix and trailing slashes
      const normalizedTarget = target.replace(/^\.\//, '').replace(/\/+$/, '');

      // Build full path
      const targetPath = normalizedTarget.startsWith('/')
        ? normalizedTarget  // Absolute path within workspace
        : `/${[...nav.currentPath, normalizedTarget].join('/')}`;  // Relative path

      const node = await getNodeByPath(currentDatabase, nav.workspace, targetPath);

      if (!node) {
        return { type: 'error', message: `No such node: ${target}` };
      }

      if (!node.has_children) {
        return { type: 'error', message: `Not a folder: ${target}` };
      }

      const newPath = targetPath.split('/').filter(s => s.length > 0);
      setNav(prev => ({
        ...prev,
        currentPath: newPath,
      }));
      return null;
    } catch (error) {
      return { type: 'error', message: `Failed to navigate: ${error instanceof Error ? error.message : String(error)}` };
    }
  };

  // Handle lstree command - only works inside a workspace
  const handleLstree = async (args: string[]): Promise<{ type: string; message: string }> => {
    if (!currentDatabase) {
      return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
    }
    if (!nav.workspace) {
      return { type: 'error', message: 'Enter a workspace first with "cd <workspace>"' };
    }

    const depth = parseInt(args[0] || '3', 10);

    try {
      // Build tree recursively using paths
      const buildTree = async (basePath: string, level: number, maxDepth: number): Promise<string[]> => {
        if (level > maxDepth) return [];

        const nodes = await listNodes(currentDatabase!, nav.workspace!, basePath);
        const lines: string[] = [];

        for (let i = 0; i < nodes.length; i++) {
          const node = nodes[i];
          const isLast = i === nodes.length - 1;
          const prefix = '  '.repeat(level);
          const connector = isLast ? '└── ' : '├── ';
          const suffix = node.has_children ? '/' : '';

          lines.push(`${prefix}${connector}${node.name}${suffix}`);

          if (node.has_children && level < maxDepth) {
            const childPath = basePath === '/' ? `/${node.name}` : `${basePath}/${node.name}`;
            const childLines = await buildTree(childPath, level + 1, maxDepth);
            lines.push(...childLines);
          }
        }

        return lines;
      };

      const pathStr = nav.currentPath.length > 0 ? nav.currentPath.join('/') : '.';
      const treeLines = await buildTree(getCurrentPathStr(), 0, depth);

      if (treeLines.length === 0) {
        return { type: 'info', message: `${pathStr}\n(empty)` };
      }

      return { type: 'info', message: `${pathStr}\n${treeLines.join('\n')}` };
    } catch (error) {
      return { type: 'error', message: `Failed to build tree: ${error instanceof Error ? error.message : String(error)}` };
    }
  };

  // Simple YAML syntax highlighter for terminal
  const highlightYaml = (yamlStr: string): string => {
    return yamlStr
      .split('\n')
      .map(line => {
        // Highlight keys (word followed by colon at start or after indent)
        if (line.match(/^(\s*)([a-zA-Z_][a-zA-Z0-9_]*):/)) {
          return line.replace(/^(\s*)([a-zA-Z_][a-zA-Z0-9_]*):(.*)$/, (_, indent, key, value) => {
            let coloredValue = value;
            // Color the value based on type
            if (value.match(/^\s*$/)) {
              // No value (nested object)
              coloredValue = '';
            } else if (value.match(/^\s*["'].*["']$/)) {
              // Quoted string
              coloredValue = `\x1b[32m${value}\x1b[0m`;
            } else if (value.match(/^\s*-?\d+\.?\d*$/)) {
              // Number
              coloredValue = `\x1b[33m${value}\x1b[0m`;
            } else if (value.match(/^\s*(true|false)$/i)) {
              // Boolean
              coloredValue = `\x1b[35m${value}\x1b[0m`;
            } else if (value.match(/^\s*(null|~)$/i)) {
              // Null
              coloredValue = `\x1b[90m${value}\x1b[0m`;
            } else {
              // Unquoted string
              coloredValue = `\x1b[32m${value}\x1b[0m`;
            }
            return `${indent}\x1b[36m${key}\x1b[0m:${coloredValue}`;
          });
        }
        // List items
        if (line.match(/^(\s*)-\s/)) {
          return line.replace(/^(\s*)-\s(.*)$/, (_, indent, value) => {
            return `${indent}\x1b[90m-\x1b[0m \x1b[32m${value}\x1b[0m`;
          });
        }
        return line;
      })
      .join('\n');
  };

  // Handle cat command - show node as YAML
  const handleCat = async (args: string[]): Promise<{ type: string; message: string }> => {
    if (!currentDatabase) {
      return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
    }
    if (!nav.workspace) {
      return { type: 'error', message: 'Enter a workspace first with "cd <workspace>"' };
    }

    const nodeName = args[0];
    if (!nodeName) {
      return { type: 'error', message: 'Usage: cat <nodename>' };
    }

    try {
      // Normalize path - strip ./ prefix
      const normalizedName = nodeName.replace(/^\.\//, '');

      // Build full path to node - handle absolute vs relative paths
      const nodePath = normalizedName.startsWith('/')
        ? normalizedName  // Absolute path within workspace
        : nav.currentPath.length > 0
          ? `/${nav.currentPath.join('/')}/${normalizedName}`
          : `/${normalizedName}`;

      const node = await getNodeFull(currentDatabase, nav.workspace, nodePath);

      if (!node) {
        return { type: 'error', message: `No such node: ${nodePath}` };
      }

      const yamlStr = yaml.stringify(node);
      const highlighted = highlightYaml(yamlStr);

      return { type: 'info', message: highlighted };
    } catch (error) {
      return { type: 'error', message: `Failed to read node: ${error instanceof Error ? error.message : String(error)}` };
    }
  };

  const handleSubmit = async (value: string) => {
    if (!value.trim()) return;

    // Reset file autocomplete
    setFileAutocomplete({ visible: false, query: '', selectedIndex: 0, atPosition: -1 });

    // Add to history
    if (value.trim() !== history[history.length - 1]) {
      setHistory(prev => [...prev, value.trim()]);
    }
    setHistoryIndex(-1);
    setSavedInput('');

    // Add command to output
    setMessages(prev => [...prev, { type: 'output', text: `${prompt}${value}` }]);

    // Parse command
    const parts = value.trim().split(/\s+/);
    const command = parts[0].toLowerCase();
    const args = parts.slice(1);

    // Special handling for /help
    if (command === '/help' || command === 'help') {
      setShowHelp(true);
      setInput('');
      return;
    }

    // Special handling for /clear
    if (command === '/clear' || command === 'clear') {
      setMessages([]);
      setInput('');
      return;
    }

    // Handle navigation commands
    if (command === 'ls') {
      setIsLoading(true);
      const result = await handleLs(args);
      setMessages(prev => [...prev, { type: result.type as any, text: result.message }]);
      setIsLoading(false);
      setInput('');
      return;
    }

    if (command === 'cd') {
      setIsLoading(true);
      const result = await handleCd(args);
      if (result) {
        setMessages(prev => [...prev, { type: result.type as any, text: result.message }]);
      }
      setIsLoading(false);
      setInput('');
      return;
    }

    if (command === 'lstree' || command === 'tree') {
      setIsLoading(true);
      const result = await handleLstree(args);
      setMessages(prev => [...prev, { type: result.type as any, text: result.message }]);
      setIsLoading(false);
      setInput('');
      return;
    }

    if (command === 'cat') {
      setIsLoading(true);
      const result = await handleCat(args);
      setMessages(prev => [...prev, { type: result.type as any, text: result.message }]);
      setIsLoading(false);
      setInput('');
      return;
    }

    if (command === 'pwd') {
      let pathStr: string;
      if (!currentDatabase) {
        pathStr = '(no database selected)';
      } else if (!nav.workspace) {
        pathStr = `/${currentDatabase}`;
      } else {
        const nodePath = nav.currentPath.length > 0 ? `/${nav.currentPath.join('/')}` : '';
        pathStr = `/${currentDatabase}/${nav.workspace}${nodePath}`;
      }
      setMessages(prev => [...prev, { type: 'info', text: pathStr }]);
      setInput('');
      return;
    }

    // Handle exit/quit without slash
    if (command === 'exit' || command === 'quit') {
      process.exit(0);
    }

    // Pass other commands to parent handler
    try {
      setIsLoading(true);
      const result = await onCommand(command, args);
      if (result) {
        setMessages(prev => [...prev, { type: result.type, text: result.message }]);
      }
    } catch (error) {
      setMessages(prev => [
        ...prev,
        { type: 'error', text: error instanceof Error ? error.message : String(error) },
      ]);
    } finally {
      setIsLoading(false);
    }

    setInput('');
  };

  if (showHelp) {
    return (
      <Box flexDirection="column">
        <HelpScreen />
        <Box marginTop={1}>
          <Text dimColor>Press Enter to continue...</Text>
        </Box>
        <Box marginTop={1}>
          <Text color="#B8754E">{prompt}</Text>
          <TextInput
            value={input}
            onChange={setInput}
            onSubmit={() => {
              setShowHelp(false);
              setInput('');
            }}
          />
        </Box>
      </Box>
    );
  }

  return (
    <Box flexDirection="column">
      {/* Message history */}
      <Box flexDirection="column">
        {messages.map((msg, idx) => (
          <Box key={idx}>
            {msg.type === 'output' && <Text dimColor>{msg.text}</Text>}
            {msg.type === 'success' && <SuccessMessage message={msg.text} />}
            {msg.type === 'error' && <ErrorMessage message={msg.text} />}
            {msg.type === 'info' && <InfoMessage message={msg.text} />}
          </Box>
        ))}
      </Box>

      {/* Loading spinner */}
      {isLoading && (
        <Box marginY={1}>
          <Spinner text="Loading..." />
        </Box>
      )}

      {/* Input prompt */}
      {!isLoading && (
        <Box flexDirection="column">
          <Box marginTop={messages.length > 0 ? 1 : 0}>
            <Text color="#B8754E">{prompt}</Text>
            <TextInput value={input} onChange={handleInputChange} onSubmit={handleSubmit} />
          </Box>

          {/* @ file autocomplete dropdown */}
          <FileAutocomplete
            query={fileAutocomplete.query}
            selectedIndex={fileAutocomplete.selectedIndex}
            visible={fileAutocomplete.visible}
          />
        </Box>
      )}

      {/* Ctrl+C hint - appears below prompt */}
      {ctrlCPressed && (
        <Box marginTop={1} paddingX={1}>
          <Text color="yellow" dimColor>
            Press Ctrl+C again to exit
          </Text>
        </Box>
      )}
    </Box>
  );
};

export default Shell;

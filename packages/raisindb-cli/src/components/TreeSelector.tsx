/**
 * Interactive tree selector for CLI
 * Allows selecting nodes from a workspace tree with keyboard navigation
 */

import React, { useState, useEffect, useCallback } from 'react';
import { Box, Text, useInput, useApp } from 'ink';
import Spinner from 'ink-spinner';
import { listWorkspaces, listNodes, type NodeInfo, type Workspace } from '../api.js';
import { loadConfig } from '../config.js';

export interface SelectedNode {
  workspace: string;
  path: string;
  name: string;
  isRecursive: boolean;
}

interface TreeSelectorProps {
  repo: string;
  onComplete: (selected: SelectedNode[]) => void;
  onCancel: () => void;
}

interface TreeNode extends NodeInfo {
  children?: TreeNode[];
  loaded?: boolean;
  level: number;
  workspace: string;
}

type FocusArea = 'workspaces' | 'tree';

export function TreeSelector({ repo, onComplete, onCancel }: TreeSelectorProps) {
  const { exit } = useApp();

  // Workspace state
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [selectedWorkspace, setSelectedWorkspace] = useState<string>('');
  const [workspaceIndex, setWorkspaceIndex] = useState(0);
  const [loadingWorkspaces, setLoadingWorkspaces] = useState(true);

  // Tree state
  const [flattenedTree, setFlattenedTree] = useState<TreeNode[]>([]);
  const [cursorIndex, setCursorIndex] = useState(0);
  const [expandedNodes, setExpandedNodes] = useState<Set<string>>(new Set());
  const [loadingNodes, setLoadingNodes] = useState(false);

  // Selection state
  const [selectedPaths, setSelectedPaths] = useState<Map<string, SelectedNode>>(new Map());

  // Focus state (toggle between workspace selector and tree)
  const [focusArea, setFocusArea] = useState<FocusArea>('workspaces');

  // Load workspaces on mount
  useEffect(() => {
    async function load() {
      try {
        setLoadingWorkspaces(true);
        const ws = await listWorkspaces(repo);
        // Filter out system workspaces
        const userWorkspaces = ws.filter(w =>
          !['packages', 'nodetypes', 'admin', 'system'].includes(w.name)
        );
        setWorkspaces(userWorkspaces);
        if (userWorkspaces.length > 0) {
          const defaultWs = userWorkspaces.find(w => w.name === 'content') || userWorkspaces[0];
          setSelectedWorkspace(defaultWs.name);
          setWorkspaceIndex(userWorkspaces.findIndex(w => w.name === defaultWs.name));
        }
      } catch (error) {
        console.error('Failed to load workspaces:', error);
      } finally {
        setLoadingWorkspaces(false);
      }
    }
    load();
  }, [repo]);

  // Load tree when workspace changes
  useEffect(() => {
    async function loadTree() {
      if (!selectedWorkspace) return;
      try {
        setLoadingNodes(true);
        const nodes = await listNodes(repo, selectedWorkspace, '/');
        const treeNodes: TreeNode[] = nodes.map(n => ({
          ...n,
          level: 0,
          workspace: selectedWorkspace,
          loaded: false,
        }));
        setFlattenedTree(treeNodes);
        setCursorIndex(0);
        setExpandedNodes(new Set());
      } catch (error) {
        console.error('Failed to load nodes:', error);
        setFlattenedTree([]);
      } finally {
        setLoadingNodes(false);
      }
    }
    loadTree();
  }, [repo, selectedWorkspace]);

  // Flatten tree based on expanded state
  const rebuildFlattenedTree = useCallback(async (nodes: TreeNode[], expandedSet: Set<string>): Promise<TreeNode[]> => {
    const result: TreeNode[] = [];

    const processNode = async (node: TreeNode) => {
      result.push(node);

      if (expandedSet.has(node.id) && node.has_children) {
        // Load children if not loaded
        if (!node.loaded && !node.children) {
          try {
            const children = await listNodes(repo, selectedWorkspace, node.path);
            node.children = children.map(c => ({
              ...c,
              level: node.level + 1,
              workspace: selectedWorkspace,
              loaded: false,
            }));
            node.loaded = true;
          } catch (error) {
            console.error('Failed to load children:', error);
          }
        }

        if (node.children) {
          for (const child of node.children) {
            await processNode(child);
          }
        }
      }
    };

    for (const node of nodes) {
      await processNode(node);
    }

    return result;
  }, [repo, selectedWorkspace]);

  // Handle expand/collapse
  const toggleExpand = useCallback(async (node: TreeNode) => {
    const newExpanded = new Set(expandedNodes);
    if (newExpanded.has(node.id)) {
      newExpanded.delete(node.id);
    } else {
      newExpanded.add(node.id);
    }
    setExpandedNodes(newExpanded);

    // Get root nodes (level 0)
    const rootNodes = flattenedTree.filter(n => n.level === 0);
    const newFlattened = await rebuildFlattenedTree(rootNodes, newExpanded);
    setFlattenedTree(newFlattened);
  }, [expandedNodes, flattenedTree, rebuildFlattenedTree]);

  // Handle selection toggle
  const toggleSelection = useCallback((node: TreeNode) => {
    const key = `${node.workspace}:${node.path}`;
    const newSelected = new Map(selectedPaths);

    if (newSelected.has(key)) {
      newSelected.delete(key);
    } else {
      newSelected.set(key, {
        workspace: node.workspace,
        path: node.path || `/${node.name}`,
        name: node.name,
        isRecursive: node.has_children || false,
      });
    }

    setSelectedPaths(newSelected);
  }, [selectedPaths]);

  // Toggle recursive mode for selected node
  const toggleRecursive = useCallback((node: TreeNode) => {
    const key = `${node.workspace}:${node.path}`;
    const current = selectedPaths.get(key);
    if (current) {
      const newSelected = new Map(selectedPaths);
      newSelected.set(key, {
        ...current,
        isRecursive: !current.isRecursive,
      });
      setSelectedPaths(newSelected);
    }
  }, [selectedPaths]);

  // Handle keyboard input
  useInput((input, key) => {
    // Global keys
    if (key.escape || (input === 'q' && !key.ctrl)) {
      onCancel();
      return;
    }

    if (key.return) {
      // Submit selection
      if (selectedPaths.size > 0) {
        onComplete(Array.from(selectedPaths.values()));
      }
      return;
    }

    // Tab to switch focus
    if (key.tab) {
      setFocusArea(prev => prev === 'workspaces' ? 'tree' : 'workspaces');
      return;
    }

    if (focusArea === 'workspaces') {
      // Workspace navigation
      if (key.upArrow || input === 'k') {
        setWorkspaceIndex(prev => Math.max(0, prev - 1));
        if (workspaces[Math.max(0, workspaceIndex - 1)]) {
          setSelectedWorkspace(workspaces[Math.max(0, workspaceIndex - 1)].name);
        }
      } else if (key.downArrow || input === 'j') {
        const newIndex = Math.min(workspaces.length - 1, workspaceIndex + 1);
        setWorkspaceIndex(newIndex);
        if (workspaces[newIndex]) {
          setSelectedWorkspace(workspaces[newIndex].name);
        }
      } else if (key.return || key.rightArrow) {
        setFocusArea('tree');
      }
    } else {
      // Tree navigation
      if (key.upArrow || input === 'k') {
        setCursorIndex(prev => Math.max(0, prev - 1));
      } else if (key.downArrow || input === 'j') {
        setCursorIndex(prev => Math.min(flattenedTree.length - 1, prev + 1));
      } else if (key.leftArrow || input === 'h') {
        // Collapse or go to parent
        const node = flattenedTree[cursorIndex];
        if (node && expandedNodes.has(node.id)) {
          toggleExpand(node);
        } else if (node && node.level > 0) {
          // Find parent and move cursor there
          const parentLevel = node.level - 1;
          for (let i = cursorIndex - 1; i >= 0; i--) {
            if (flattenedTree[i].level === parentLevel) {
              setCursorIndex(i);
              break;
            }
          }
        } else {
          setFocusArea('workspaces');
        }
      } else if (key.rightArrow || input === 'l') {
        // Expand
        const node = flattenedTree[cursorIndex];
        if (node && node.has_children && !expandedNodes.has(node.id)) {
          toggleExpand(node);
        }
      } else if (input === ' ') {
        // Toggle selection
        const node = flattenedTree[cursorIndex];
        if (node) {
          toggleSelection(node);
        }
      } else if (input === 'r') {
        // Toggle recursive
        const node = flattenedTree[cursorIndex];
        if (node) {
          toggleRecursive(node);
        }
      }
    }
  });

  // Render workspace list
  const renderWorkspaces = () => (
    <Box flexDirection="column" borderStyle="round" borderColor={focusArea === 'workspaces' ? 'cyan' : 'gray'} paddingX={1}>
      <Text bold color="cyan">Workspaces</Text>
      {loadingWorkspaces ? (
        <Box>
          <Text color="gray"><Spinner type="dots" /> Loading...</Text>
        </Box>
      ) : (
        workspaces.map((ws, i) => (
          <Text
            key={ws.name}
            color={i === workspaceIndex ? 'cyan' : 'white'}
            inverse={i === workspaceIndex && focusArea === 'workspaces'}
          >
            {i === workspaceIndex ? '> ' : '  '}{ws.name}
          </Text>
        ))
      )}
    </Box>
  );

  // Render tree
  const renderTree = () => (
    <Box flexDirection="column" borderStyle="round" borderColor={focusArea === 'tree' ? 'cyan' : 'gray'} paddingX={1} flexGrow={1}>
      <Text bold color="cyan">Content ({selectedPaths.size} selected)</Text>
      {loadingNodes ? (
        <Box>
          <Text color="gray"><Spinner type="dots" /> Loading...</Text>
        </Box>
      ) : flattenedTree.length === 0 ? (
        <Text color="gray">No content in this workspace</Text>
      ) : (
        <Box flexDirection="column" height={15} overflow="hidden">
          {flattenedTree.slice(Math.max(0, cursorIndex - 7), cursorIndex + 8).map((node, displayIndex) => {
            const actualIndex = Math.max(0, cursorIndex - 7) + displayIndex;
            const isAtCursor = actualIndex === cursorIndex;
            const key = `${node.workspace}:${node.path}`;
            const isSelected = selectedPaths.has(key);
            const selectedNode = selectedPaths.get(key);
            const indent = '  '.repeat(node.level);
            const expandIcon = node.has_children
              ? (expandedNodes.has(node.id) ? 'v ' : '> ')
              : '  ';
            const checkIcon = isSelected ? '[x]' : '[ ]';
            const recursiveTag = isSelected && node.has_children
              ? (selectedNode?.isRecursive ? ' (recursive)' : ' (only this)')
              : '';

            return (
              <Text
                key={node.id}
                color={isSelected ? 'green' : 'white'}
                inverse={isAtCursor && focusArea === 'tree'}
              >
                {indent}{expandIcon}{checkIcon} {node.name}
                <Text color="gray" dimColor>{recursiveTag}</Text>
              </Text>
            );
          })}
        </Box>
      )}
    </Box>
  );

  // Render help
  const renderHelp = () => (
    <Box marginTop={1}>
      <Text color="gray">
        Tab: switch panel | Space: select | r: toggle recursive | Enter: confirm | q/Esc: cancel
      </Text>
    </Box>
  );

  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text bold>Select content to include in package</Text>
      </Box>

      <Box>
        <Box width={25}>
          {renderWorkspaces()}
        </Box>
        <Box marginLeft={1} flexGrow={1}>
          {renderTree()}
        </Box>
      </Box>

      {renderHelp()}
    </Box>
  );
}

export default TreeSelector;

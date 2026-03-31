import React from 'react';
import { Box, Text } from 'ink';
import fs from 'fs';
import path from 'path';

export interface FileSuggestion {
  name: string;
  isDirectory: boolean;
  fullPath: string;
}

export interface FileAutocompleteProps {
  query: string;
  selectedIndex: number;
  visible: boolean;
  maxItems?: number;
}

// Get filesystem suggestions for a path query
export const getFileSuggestions = (query: string): FileSuggestion[] => {
  try {
    // Default to current directory if empty
    let searchPath = query || './';

    // Normalize the path
    const isRelative = !path.isAbsolute(searchPath);
    const basePath = isRelative ? process.cwd() : '';

    // Find the directory to search and the partial filename
    let dirToSearch: string;
    let partial: string;

    const lastSlash = searchPath.lastIndexOf('/');
    if (lastSlash === -1) {
      // No slash - search current directory
      dirToSearch = basePath || '.';
      partial = searchPath;
    } else {
      // Has slash
      partial = searchPath.substring(lastSlash + 1);
      const dirPart = searchPath.substring(0, lastSlash) || '.';
      dirToSearch = isRelative ? path.join(basePath, dirPart) : dirPart;
    }

    // Check if directory exists
    if (!fs.existsSync(dirToSearch)) {
      return [];
    }

    const entries = fs.readdirSync(dirToSearch, { withFileTypes: true });

    // Filter and map entries
    const suggestions: FileSuggestion[] = entries
      .filter((entry) => !entry.name.startsWith('.')) // Skip hidden files
      .filter((entry) =>
        partial === '' || entry.name.toLowerCase().startsWith(partial.toLowerCase())
      )
      .map((entry) => ({
        name: entry.isDirectory() ? `${entry.name}/` : entry.name,
        isDirectory: entry.isDirectory(),
        fullPath: lastSlash === -1
          ? entry.isDirectory() ? `${entry.name}/` : entry.name
          : `${searchPath.substring(0, lastSlash + 1)}${entry.isDirectory() ? `${entry.name}/` : entry.name}`,
      }))
      // Sort: directories first, then alphabetically
      .sort((a, b) => {
        if (a.isDirectory && !b.isDirectory) return -1;
        if (!a.isDirectory && b.isDirectory) return 1;
        return a.name.localeCompare(b.name);
      });

    return suggestions;
  } catch {
    return [];
  }
};

// Find the longest common prefix among suggestions
export const findCommonPrefix = (suggestions: FileSuggestion[]): string => {
  if (suggestions.length === 0) return '';
  if (suggestions.length === 1) return suggestions[0].fullPath;

  const names = suggestions.map((s) => s.fullPath);
  let prefix = names[0];

  for (let i = 1; i < names.length; i++) {
    while (!names[i].toLowerCase().startsWith(prefix.toLowerCase())) {
      prefix = prefix.slice(0, -1);
      if (prefix === '') return '';
    }
    // Use the casing from first match
    prefix = names[0].slice(0, prefix.length);
  }

  return prefix;
};

const FileAutocomplete: React.FC<FileAutocompleteProps> = ({
  query,
  selectedIndex,
  visible,
  maxItems = 8,
}) => {
  if (!visible) return null;

  const suggestions = getFileSuggestions(query);

  if (suggestions.length === 0) {
    return (
      <Box
        flexDirection="column"
        borderStyle="single"
        borderColor="gray"
        paddingX={1}
        marginLeft={2}
      >
        <Text dimColor italic>
          No matches found
        </Text>
      </Box>
    );
  }

  // Limit displayed items and handle scrolling
  const displayedSuggestions = suggestions.slice(0, maxItems);
  const hasMore = suggestions.length > maxItems;

  return (
    <Box
      flexDirection="column"
      borderStyle="single"
      borderColor="cyan"
      paddingX={1}
      marginLeft={2}
    >
      {displayedSuggestions.map((suggestion, index) => {
        const isSelected = index === selectedIndex;
        return (
          <Box key={suggestion.fullPath}>
            <Text color={isSelected ? 'cyan' : undefined} bold={isSelected}>
              {isSelected ? '> ' : '  '}
            </Text>
            <Text
              color={suggestion.isDirectory ? 'blue' : undefined}
              bold={isSelected}
            >
              {suggestion.name}
            </Text>
          </Box>
        );
      })}
      {hasMore && (
        <Text dimColor italic>
          ...and {suggestions.length - maxItems} more
        </Text>
      )}
    </Box>
  );
};

export default FileAutocomplete;

/**
 * Interactive package selector for clone command
 */

import React, { useState, useEffect } from 'react';
import { Box, Text, useInput, useApp } from 'ink';
import Spinner from 'ink-spinner';
import SelectInput from 'ink-select-input';
import { listPackages, type PackageSummary } from '../api.js';

export interface PackageSelectorProps {
  repo: string;
  onSelect: (packageName: string) => void;
  onCancel?: () => void;
}

interface PackageItem {
  label: string;
  value: string;
  package: PackageSummary;
}

export function PackageSelector({ repo, onSelect, onCancel }: PackageSelectorProps) {
  const { exit } = useApp();
  const [packages, setPackages] = useState<PackageSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');

  useEffect(() => {
    loadPackages();
  }, [repo]);

  async function loadPackages() {
    setLoading(true);
    setError(null);
    try {
      const pkgs = await listPackages(repo);
      setPackages(pkgs);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load packages');
    } finally {
      setLoading(false);
    }
  }

  // Handle escape key to cancel
  useInput((input, key) => {
    if (key.escape) {
      if (onCancel) {
        onCancel();
      } else {
        exit();
      }
    }
  });

  // Convert packages to SelectInput items
  const items: PackageItem[] = packages
    .filter(pkg => {
      if (!filter) return true;
      const searchTerm = filter.toLowerCase();
      return (
        pkg.name.toLowerCase().includes(searchTerm) ||
        (pkg.title?.toLowerCase().includes(searchTerm) ?? false)
      );
    })
    .map(pkg => ({
      label: `${pkg.name} (v${pkg.version})${pkg.installed ? ' [installed]' : ''}`,
      value: pkg.id,  // Use ID as unique key to avoid React duplicate key warnings
      package: pkg,
    }));

  function handleSelect(item: { label: string; value: string }) {
    // Find the package by ID and return its name
    const selectedPkg = packages.find(p => p.id === item.value);
    onSelect(selectedPkg?.name || item.value);
  }

  if (loading) {
    return (
      <Box flexDirection="column" padding={1}>
        <Box>
          <Text color="cyan">
            <Spinner type="dots" />
          </Text>
          <Text> Loading packages from {repo}...</Text>
        </Box>
      </Box>
    );
  }

  if (error) {
    return (
      <Box flexDirection="column" padding={1}>
        <Text color="red">Error: {error}</Text>
        <Text dimColor>Press Escape to exit</Text>
      </Box>
    );
  }

  if (packages.length === 0) {
    return (
      <Box flexDirection="column" padding={1}>
        <Text color="yellow">No packages found in repository "{repo}"</Text>
        <Text dimColor>Press Escape to exit</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column" padding={1}>
      <Box marginBottom={1}>
        <Text bold color="cyan">Select a package to clone:</Text>
      </Box>

      <Box marginBottom={1}>
        <Text dimColor>Repository: </Text>
        <Text>{repo}</Text>
        <Text dimColor> ({packages.length} packages)</Text>
      </Box>

      {items.length === 0 ? (
        <Text color="yellow">No packages match "{filter}"</Text>
      ) : (
        <SelectInput
          items={items}
          onSelect={handleSelect}
          itemComponent={PackageItemComponent}
        />
      )}

      <Box marginTop={1}>
        <Text dimColor>Use arrow keys to navigate, Enter to select, Escape to cancel</Text>
      </Box>
    </Box>
  );
}

// Custom item component to show package details
function PackageItemComponent({
  isSelected,
  label,
}: {
  isSelected?: boolean;
  label: string;
}) {
  return (
    <Text color={isSelected ? 'cyan' : undefined}>
      {isSelected ? '> ' : '  '}
      {label}
    </Text>
  );
}

export default PackageSelector;

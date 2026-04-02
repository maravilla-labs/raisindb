import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import SelectInput from 'ink-select-input';
import TextInput from 'ink-text-input';
import Spinner from './Spinner.js';
import { SuccessMessage, ErrorMessage } from './StatusMessages.js';
import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import AdmZip from 'adm-zip';
import {
  validatePackageDirectory,
  getValidationSummary,
} from '../wasm/schema-validator.js';
import type { PackageValidationResults, ValidationError } from '../wasm/types.js';

interface PackageCreatorProps {
  onExit: () => void;
  onSuccess?: (packagePath: string) => void;
}

type Step = 'select-dir' | 'confirm' | 'validating' | 'validation-results' | 'creating' | 'done' | 'error';

interface DirectoryItem {
  label: string;
  value: string;
}

interface PackageManifest {
  name: string;
  version: string;
  description?: string;
  author?: string;
  files?: string[];
}

const PackageCreator: React.FC<PackageCreatorProps> = ({ onExit, onSuccess }) => {
  const [step, setStep] = useState<Step>('select-dir');
  const [currentDir, setCurrentDir] = useState(process.cwd());
  const [directories, setDirectories] = useState<DirectoryItem[]>([]);
  const [selectedDir, setSelectedDir] = useState<string | null>(null);
  const [manifest, setManifest] = useState<PackageManifest | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [createdPackage, setCreatedPackage] = useState<string | null>(null);
  const [validationResults, setValidationResults] = useState<PackageValidationResults | null>(null);

  // Load directories when currentDir changes
  useEffect(() => {
    try {
      const entries = fs.readdirSync(currentDir, { withFileTypes: true });
      const dirs: DirectoryItem[] = [];

      // Add parent directory option
      const parentDir = path.dirname(currentDir);
      if (parentDir !== currentDir) {
        dirs.push({ label: '📁 ..', value: parentDir });
      }

      // Add subdirectories
      for (const entry of entries) {
        if (entry.isDirectory() && !entry.name.startsWith('.')) {
          const fullPath = path.join(currentDir, entry.name);
          // Check if directory has manifest.yaml (backend expects manifest.yaml)
          const hasManifest = fs.existsSync(path.join(fullPath, 'manifest.yaml')) ||
                              fs.existsSync(path.join(fullPath, 'manifest.yml'));

          const icon = hasManifest ? '📦' : '📁';
          dirs.push({ label: `${icon} ${entry.name}`, value: fullPath });
        }
      }

      // Add "Use current directory" option if it has a manifest
      const currentHasManifest = fs.existsSync(path.join(currentDir, 'manifest.yaml')) ||
                                  fs.existsSync(path.join(currentDir, 'manifest.yml'));
      if (currentHasManifest) {
        dirs.unshift({ label: '✅ Use this directory', value: currentDir });
      }

      // Add cancel option
      dirs.push({ label: '❌ Cancel', value: '__cancel__' });

      setDirectories(dirs);
    } catch (err) {
      setError(`Failed to read directory: ${err instanceof Error ? err.message : String(err)}`);
    }
  }, [currentDir]);

  const handleSelect = (item: DirectoryItem) => {
    if (item.value === '__cancel__') {
      onExit();
      return;
    }

    // Check if this is a package directory (has manifest)
    const hasManifest = fs.existsSync(path.join(item.value, 'manifest.yaml')) ||
                        fs.existsSync(path.join(item.value, 'manifest.yml'));

    if (hasManifest) {
      // Read manifest and go to confirm step
      try {
        const manifestPath = fs.existsSync(path.join(item.value, 'manifest.yaml'))
          ? path.join(item.value, 'manifest.yaml')
          : path.join(item.value, 'manifest.yml');

        const manifestContent = fs.readFileSync(manifestPath, 'utf-8');
        const parsed = yaml.parse(manifestContent) as PackageManifest;

        setSelectedDir(item.value);
        setManifest(parsed);
        setStep('confirm');
      } catch (err) {
        setError(`Failed to read manifest: ${err instanceof Error ? err.message : String(err)}`);
        setStep('error');
      }
    } else {
      // Navigate into the directory
      setCurrentDir(item.value);
    }
  };

  const handleConfirm = async () => {
    if (!selectedDir || !manifest) return;

    // First, validate the package
    setStep('validating');

    try {
      const results = await validatePackageDirectory(selectedDir);
      setValidationResults(results);
      setStep('validation-results');
    } catch (err) {
      setError(`Validation failed: ${err instanceof Error ? err.message : String(err)}`);
      setStep('error');
    }
  };

  const handleCreatePackage = async () => {
    if (!selectedDir || !manifest) return;

    setStep('creating');

    try {
      const outputPath = path.join(process.cwd(), `${manifest.name}-${manifest.version}.rap`);

      // Create ZIP archive
      createZipPackage(selectedDir, outputPath);

      setCreatedPackage(outputPath);
      setStep('done');

      if (onSuccess) {
        onSuccess(outputPath);
      }
    } catch (err) {
      setError(`Failed to create package: ${err instanceof Error ? err.message : String(err)}`);
      setStep('error');
    }
  };

  // Create a ZIP package from the source directory
  const createZipPackage = (sourceDir: string, outputPath: string): void => {
    const zip = new AdmZip();
    zip.addLocalFolder(sourceDir);
    zip.writeZip(outputPath);
  };

  // Helper to collect files
  const collectFiles = async (folderPath: string): Promise<Record<string, string>> => {
    const files: Record<string, string> = {};

    function walkDir(dir: string, baseDir: string = dir) {
      const entries = fs.readdirSync(dir, { withFileTypes: true });

      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);
        const relativePath = path.relative(baseDir, fullPath);

        // Skip node_modules, .git, etc.
        if (entry.name.startsWith('.') || entry.name === 'node_modules') {
          continue;
        }

        if (entry.isDirectory()) {
          walkDir(fullPath, baseDir);
        } else if (entry.isFile()) {
          try {
            const content = fs.readFileSync(fullPath, 'utf-8');
            files[relativePath] = content;
          } catch {
            files[relativePath] = `[Binary file: ${relativePath}]`;
          }
        }
      }
    }

    walkDir(folderPath);
    return files;
  };

  // Render based on current step
  if (step === 'error' && error) {
    return (
      <Box flexDirection="column">
        <ErrorMessage message={error} />
        <Box marginTop={1}>
          <Text dimColor>Press Enter to go back...</Text>
        </Box>
        <TextInput value="" onChange={() => {}} onSubmit={onExit} />
      </Box>
    );
  }

  if (step === 'done' && createdPackage) {
    return (
      <Box flexDirection="column">
        <SuccessMessage message={`Package created: ${createdPackage}`} />
        <Box marginTop={1}>
          <Text dimColor>Press Enter to continue...</Text>
        </Box>
        <TextInput value="" onChange={() => {}} onSubmit={onExit} />
      </Box>
    );
  }

  if (step === 'creating') {
    return (
      <Box flexDirection="column">
        <Spinner text={`Creating package ${manifest?.name}...`} />
      </Box>
    );
  }

  if (step === 'validating') {
    return (
      <Box flexDirection="column">
        <Spinner text="Validating package..." />
      </Box>
    );
  }

  if (step === 'validation-results' && validationResults) {
    const summary = getValidationSummary(validationResults);
    const hasErrors = summary.hasErrors;

    // Build options based on validation results
    const resultOptions = hasErrors
      ? [
          { label: '🔄 Fix and retry', value: 'retry' },
          { label: '❌ Cancel', value: 'cancel' },
        ]
      : [
          { label: '✅ Continue with package creation', value: 'continue' },
          { label: '❌ Cancel', value: 'cancel' },
        ];

    return (
      <Box flexDirection="column">
        <Box marginBottom={1}>
          <Text bold color={hasErrors ? 'red' : '#D97706'}>
            Validation {hasErrors ? 'Failed' : 'Complete'}
          </Text>
        </Box>

        {/* Show validation results */}
        {Object.entries(validationResults).map(([filePath, result]) => {
          if (result.errors.length === 0 && result.warnings.length === 0) return null;

          return (
            <Box key={filePath} flexDirection="column" marginBottom={1}>
              <Text color="cyan">{filePath}:</Text>

              {result.errors.map((err, i) => (
                <Box key={`err-${i}`} marginLeft={2} flexDirection="column">
                  <Text color="red">
                    error [{err.error_code}] {err.message}
                  </Text>
                  {err.field_path && (
                    <Box marginLeft={2}><Text dimColor>at {err.field_path}</Text></Box>
                  )}
                  {err.suggested_fix?.new_value && (
                    <Box marginLeft={2}>
                      <Text color="yellow">suggestion: {err.suggested_fix.description}</Text>
                    </Box>
                  )}
                </Box>
              ))}

              {result.warnings.map((warn, i) => (
                <Box key={`warn-${i}`} marginLeft={2} flexDirection="column">
                  <Text color="yellow">
                    warn [{warn.error_code}] {warn.message}
                  </Text>
                  {warn.field_path && (
                    <Box marginLeft={2}><Text dimColor>at {warn.field_path}</Text></Box>
                  )}
                </Box>
              ))}
            </Box>
          );
        })}

        {/* Summary */}
        <Box marginTop={1} marginBottom={1}>
          <Text>
            Summary: <Text color={hasErrors ? 'red' : 'green'}>{summary.errorCount} error(s)</Text>
            , <Text color={summary.warningCount > 0 ? 'yellow' : 'green'}>{summary.warningCount} warning(s)</Text>
          </Text>
        </Box>

        {hasErrors && (
          <Box marginBottom={1}>
            <Text color="red">
              Fix errors before creating package.
            </Text>
          </Box>
        )}

        <SelectInput
          items={resultOptions}
          onSelect={(item) => {
            if (item.value === 'continue') {
              handleCreatePackage();
            } else if (item.value === 'retry') {
              // Go back to confirm step to retry
              setValidationResults(null);
              setStep('confirm');
            } else {
              onExit();
            }
          }}
        />
      </Box>
    );
  }

  if (step === 'confirm' && manifest && selectedDir) {
    const confirmOptions = [
      { label: '✅ Yes, create package', value: 'yes' },
      { label: '❌ No, cancel', value: 'no' },
    ];

    return (
      <Box flexDirection="column">
        <Box marginBottom={1}>
          <Text bold color="#D97706">Create Package</Text>
        </Box>

        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text dimColor>Name:    </Text>
            <Text color="green">{manifest.name}</Text>
          </Text>
          <Text>
            <Text dimColor>Version: </Text>
            <Text color="green">{manifest.version}</Text>
          </Text>
          {manifest.description && (
            <Text>
              <Text dimColor>Desc:    </Text>
              <Text>{manifest.description}</Text>
            </Text>
          )}
          <Text>
            <Text dimColor>Source:  </Text>
            <Text>{selectedDir}</Text>
          </Text>
        </Box>

        <Box marginTop={1} flexDirection="column">
          <Box marginBottom={1}>
            <Text dimColor>Continue?</Text>
          </Box>
          <SelectInput
            items={confirmOptions}
            onSelect={(item) => {
              if (item.value === 'yes') {
                handleConfirm();
              } else {
                onExit();
              }
            }}
          />
        </Box>
      </Box>
    );
  }

  // Default: directory selection
  return (
    <Box flexDirection="column">
      <Box marginBottom={1}>
        <Text bold color="#D97706">Select Package Directory</Text>
      </Box>

      <Box marginBottom={1}>
        <Text dimColor>Current: </Text>
        <Text color="#EA580C">{currentDir}</Text>
      </Box>

      <Box marginBottom={1}>
        <Text dimColor>📦 = has manifest.yaml  |  📁 = directory</Text>
      </Box>

      <SelectInput
        items={directories}
        onSelect={handleSelect}
      />
    </Box>
  );
};

export default PackageCreator;

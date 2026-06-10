import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import { doctorWorkflowData } from './doctor.js';
import { validatePackageFlows, mergeFlowResults } from './package-doctor.js';
import {
  validatePackageDirectory,
  getValidationSummary,
  collectPackageFiles,
} from '../wasm/schema-validator.js';
import type { PackageValidationResults } from '../wasm/types.js';

function makeTempDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-flow-pkg-test-'));
}

function writeFile(base: string, relPath: string, content: string) {
  const full = path.join(base, relPath);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, content);
}

const GOOD_FLOW_YAML = `node_type: raisin:Flow
properties:
  name: good-flow
  title: Good Flow
  enabled: true
  workflow_data:
    version: 1
    error_strategy: fail_fast
    nodes:
      - id: fetch_data
        node_type: raisin:FlowStep
        properties:
          action: Fetch the data
          lua_script: "return { items = {} }"
      - id: loop_items
        node_type: raisin:FlowContainer
        container_type: loop
        loop:
          over: "\${steps.fetch_data.items}"
          item: item
          max_iterations: 10
        children:
          - id: process_item
            node_type: raisin:FlowStep
            properties:
              action: "Process {{ item.name }}"
              lua_script: "return {}"
`;

// Two doctor ERRORS: hyphenated step id used with dot access in a template
// (TEMPLATE_HYPHENATED_STEP_PATH) and a loop container without loop.over
// (LOOP_MISSING_OVER).
const BROKEN_FLOW_YAML = `node_type: raisin:Flow
properties:
  name: broken-flow
  title: Broken Flow
  enabled: true
  workflow_data:
    version: 1
    nodes:
      - id: fetch-data
        node_type: raisin:FlowStep
        properties:
          action: Fetch the data
          lua_script: "return { items = {} }"
      - id: use_data
        node_type: raisin:FlowStep
        properties:
          action: "Use {{ steps.fetch-data.items }}"
          lua_script: "return {}"
      - id: loop_items
        node_type: raisin:FlowContainer
        container_type: loop
        loop:
          item: item
        children:
          - id: process_item
            node_type: raisin:FlowStep
            properties:
              action: Process one item
              lua_script: "return {}"
`;

const MANIFEST_YAML = `name: flow-doctor-fixture
version: 0.0.1
title: Flow Doctor Fixture
description: Test fixture package for flow doctor integration
`;

describe('doctorWorkflowData (pure shared entry point)', () => {
  it('returns no findings for a clean designer definition', () => {
    const result = doctorWorkflowData({
      version: 1,
      nodes: [
        {
          id: 'do_it',
          node_type: 'raisin:FlowStep',
          properties: { action: 'Do it', lua_script: 'return {}' },
        },
      ],
    });
    expect(result.format).toBe('designer');
    expect(result.findings).toEqual([]);
  });

  it('reports doctor errors for a broken definition', () => {
    const result = doctorWorkflowData({
      version: 1,
      nodes: [
        {
          id: 'loop_items',
          node_type: 'raisin:FlowContainer',
          container_type: 'loop',
          loop: { item: 'item' },
          children: [
            {
              id: 'child',
              node_type: 'raisin:FlowStep',
              properties: { action: 'Use {{ steps.fetch-data.items }}', lua_script: 'return {}' },
            },
          ],
        },
      ],
    });
    const codes = result.findings.map((f) => f.code);
    expect(codes).toContain('LOOP_MISSING_OVER');
    expect(codes).toContain('TEMPLATE_HYPHENATED_STEP_PATH');
  });

  it('flags unrecognizable workflow_data as INVALID_WORKFLOW_DATA', () => {
    const result = doctorWorkflowData({ not_a_flow: true });
    expect(result.format).toBe('invalid');
    expect(result.findings).toHaveLength(1);
    expect(result.findings[0].code).toBe('INVALID_WORKFLOW_DATA');
    expect(result.findings[0].severity).toBe('error');
  });

  it('skips runtime-format definitions', () => {
    const result = doctorWorkflowData({
      nodes: [{ id: 'a', step_type: 'function' }],
    });
    expect(result.format).toBe('runtime');
    expect(result.findings).toEqual([]);
  });
});

describe('validatePackageFlows', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it('returns no results for a package with a clean flow', () => {
    writeFile(tmpDir, 'manifest.yaml', MANIFEST_YAML);
    writeFile(tmpDir, 'content/functions/flows/good-flow/.node.yaml', GOOD_FLOW_YAML);

    const files = collectPackageFiles(tmpDir);
    const results = validatePackageFlows(tmpDir, files);
    expect(Object.keys(results)).toEqual([]);
  });

  it('reports doctor errors keyed by the flow file path', () => {
    writeFile(tmpDir, 'manifest.yaml', MANIFEST_YAML);
    writeFile(tmpDir, 'content/functions/flows/broken-flow/.node.yaml', BROKEN_FLOW_YAML);

    const files = collectPackageFiles(tmpDir);
    const results = validatePackageFlows(tmpDir, files);

    const key = 'content/functions/flows/broken-flow/.node.yaml';
    expect(Object.keys(results)).toEqual([key]);
    expect(results[key].success).toBe(false);
    expect(results[key].file_type).toBe('content');

    const errorCodes = results[key].errors.map((e) => e.error_code);
    expect(errorCodes).toContain('LOOP_MISSING_OVER');
    expect(errorCodes).toContain('TEMPLATE_HYPHENATED_STEP_PATH');
    // node id is part of the message for debuggability
    const loopError = results[key].errors.find((e) => e.error_code === 'LOOP_MISSING_OVER');
    expect(loopError?.message).toContain('loop_items');
  });

  it('maps doctor warnings to validation warnings (not errors)', () => {
    const warnOnlyFlow = `node_type: raisin:Flow
properties:
  name: warn-flow
  workflow_data:
    version: 1
    nodes:
      - id: only_step
        node_type: raisin:FlowStep
        properties:
          action: "Use {{ bogus_root.value }}"
          lua_script: "return {}"
`;
    writeFile(tmpDir, 'content/functions/flows/warn-flow/.node.yaml', warnOnlyFlow);

    const files = collectPackageFiles(tmpDir);
    const results = validatePackageFlows(tmpDir, files);

    const key = 'content/functions/flows/warn-flow/.node.yaml';
    expect(results[key]).toBeDefined();
    expect(results[key].success).toBe(true);
    expect(results[key].errors).toEqual([]);
    expect(results[key].warnings.map((w) => w.error_code)).toContain('TEMPLATE_UNKNOWN_ROOT');
  });

  it('ignores non-flow content and flow nodes without workflow_data', () => {
    writeFile(tmpDir, 'content/pages/home/.node.yaml', 'node_type: raisin:Page\nproperties:\n  title: Home\n');
    writeFile(
      tmpDir,
      'content/functions/flows/draft/.node.yaml',
      'node_type: raisin:Flow\nproperties:\n  name: draft\n'
    );

    const files = collectPackageFiles(tmpDir);
    const results = validatePackageFlows(tmpDir, files);
    expect(Object.keys(results)).toEqual([]);
  });
});

describe('mergeFlowResults', () => {
  it('appends flow findings to existing file results and recomputes success', () => {
    const base: PackageValidationResults = {
      'content/a/.node.yaml': {
        success: true,
        file_type: 'content',
        errors: [],
        warnings: [],
      },
    };
    const flow: PackageValidationResults = {
      'content/a/.node.yaml': {
        success: false,
        file_type: 'content',
        errors: [
          {
            file_path: 'content/a/.node.yaml',
            field_path: 'workflow_data',
            error_code: 'LOOP_MISSING_OVER',
            message: 'missing over',
            severity: 'error',
            fix_type: 'manual',
          },
        ],
        warnings: [],
      },
      'content/b/.node.yaml': {
        success: true,
        file_type: 'content',
        errors: [],
        warnings: [],
      },
    };

    const merged = mergeFlowResults(base, flow);
    expect(merged['content/a/.node.yaml'].success).toBe(false);
    expect(merged['content/a/.node.yaml'].errors).toHaveLength(1);
    expect(merged['content/b/.node.yaml']).toBeDefined();
  });
});

describe('validatePackageDirectory (WASM schema + flow doctor, full gate)', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = makeTempDir();
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it('passes a clean package containing a valid flow', async () => {
    writeFile(tmpDir, 'manifest.yaml', MANIFEST_YAML);
    writeFile(tmpDir, 'content/functions/flows/good-flow/.node.yaml', GOOD_FLOW_YAML);

    const results = await validatePackageDirectory(tmpDir);
    const summary = getValidationSummary(results);
    expect(summary.hasErrors).toBe(false);
    expect(summary.errorCount).toBe(0);
  });

  it('fails validation when a flow has doctor errors', async () => {
    writeFile(tmpDir, 'manifest.yaml', MANIFEST_YAML);
    writeFile(tmpDir, 'content/functions/flows/good-flow/.node.yaml', GOOD_FLOW_YAML);
    writeFile(tmpDir, 'content/functions/flows/broken-flow/.node.yaml', BROKEN_FLOW_YAML);

    const results = await validatePackageDirectory(tmpDir);
    const summary = getValidationSummary(results);
    expect(summary.hasErrors).toBe(true);
    expect(summary.filesWithErrors).toContain('content/functions/flows/broken-flow/.node.yaml');
    expect(summary.filesWithErrors).not.toContain('content/functions/flows/good-flow/.node.yaml');

    const allErrorCodes = Object.values(results).flatMap((r) =>
      r.errors.map((e) => e.error_code)
    );
    expect(allErrorCodes).toContain('LOOP_MISSING_OVER');
    expect(allErrorCodes).toContain('TEMPLATE_HYPHENATED_STEP_PATH');
  });
});

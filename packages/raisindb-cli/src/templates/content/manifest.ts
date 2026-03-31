import type { TemplateVars } from '../types.js';

export function manifest(vars: TemplateVars): string {
  return `name: {{packageName}}
version: 0.1.0
title: {{packageName}}
description: {{description}}
author: ""
license: MIT
icon: package
color: "#6366f1"
keywords:
  - raisindb

provides:
  nodetypes: []
    # - {{namespace}}:MyType
  mixins: []
    # - {{namespace}}:MyMixin
  archetypes: []
    # - {{namespace}}:MyArchetype
  elementtypes: []
    # - {{namespace}}:MyElement
  workspaces:
    - {{workspace}}
  functions: []
    # - /lib/{{namespace}}/my-function
  triggers: []
    # - /triggers/on-my-event
`;
}

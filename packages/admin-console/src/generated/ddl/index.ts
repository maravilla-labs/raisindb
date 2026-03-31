// Auto-generated index file for DDL types
// Re-export all generated types

export * from './PropertyTypeDef';
export * from './IndexTypeDef';
export * from './DefaultValue';
export * from './PropertyDef';
export * from './KeywordInfo';
export * from './KeywordCategory';
export * from './DdlKeywords';

// Import and re-export keywords JSON with proper typing
import ddlKeywordsJson from './ddl-keywords.json';
import type { DdlKeywords } from './DdlKeywords';

// Cast to proper type since JSON import loses type info
export const ddlKeywords = ddlKeywordsJson as DdlKeywords;

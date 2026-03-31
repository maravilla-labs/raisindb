import type { KeywordCategory } from './KeywordCategory';

export type KeywordInfo = { 
/**
 * The keyword itself (e.g., "CREATE")
 */
keyword: string, 
/**
 * Category for grouping (e.g., "Statement", "Clause", "Modifier")
 */
category: KeywordCategory, 
/**
 * Human-readable description for hover tooltips
 */
description: string, 
/**
 * Optional syntax pattern
 */
syntax: string | null, 
/**
 * Optional example SQL
 */
example: string | null, };
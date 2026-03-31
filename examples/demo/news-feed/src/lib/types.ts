// Base path for the news application in the database
export const BASE_PATH = '/superbigshit';
export const ARTICLES_PATH = `${BASE_PATH}/articles`;
export const TAGS_PATH = `${BASE_PATH}/tags`;

// RaisinDB Reference type - points to another node
export interface RaisinReference {
	'raisin:ref': string;
	'raisin:workspace': string;
	'raisin:path': string;
}

// Tag node properties
export interface TagProperties {
	label: string;
	icon?: string;
	color?: string;
}

// Tag node structure
export interface TagNode {
	id: string;
	path: string;
	name: string;
	node_type: string;
	properties: TagProperties;
	children?: TagNode[];
}

export interface ArticleProperties {
	title: string;
	slug: string;
	excerpt: string;
	body: string;
	keywords: string[];
	tags: RaisinReference[];
	featured: boolean;
	status: 'draft' | 'published';
	publishing_date?: string;
	views: number;
	author: string;
	imageUrl?: string;
	category?: string;
	connections?: ArticleConnection[]; // Graph connections to other articles
}

export interface Article {
	id: string;
	path: string;
	name: string;
	node_type: string;
	properties: ArticleProperties;
	created_at: string;
	updated_at: string;
}

export interface CategoryProperties {
	label: string;
	color: string;
	order: number;
}

export interface Category {
	id: string;
	path: string;
	name: string;
	slug: string;
	properties: CategoryProperties;
}

// Convert a database path to a URL path (remove base path)
// e.g., /superbigshit/articles/tech/my-article -> /articles/tech/my-article
export function pathToUrl(dbPath?: string | null): string {
	if (!dbPath) {
		return '/';
	}
	if (dbPath.startsWith(BASE_PATH)) {
		return dbPath.slice(BASE_PATH.length) || '/';
	}
	return dbPath;
}

// Convert a URL path to a database path (add base path)
// e.g., /articles/tech/my-article -> /superbigshit/articles/tech/my-article
export function urlToPath(urlPath: string): string {
	if (urlPath.startsWith('/articles')) {
		return `${BASE_PATH}${urlPath}`;
	}
	return `${ARTICLES_PATH}${urlPath}`;
}

// Extract category slug from article path
// e.g., /superbigshit/articles/tech/my-article -> tech
export function getCategoryFromPath(path: string): string {
	const parts = path.split('/');
	// Path format: /superbigshit/articles/{category}/{slug}
	const articlesIndex = parts.indexOf('articles');
	if (articlesIndex !== -1 && parts.length > articlesIndex + 1) {
		return parts[articlesIndex + 1];
	}
	return '';
}

// Extract article slug from path
// e.g., /superbigshit/articles/tech/my-article -> my-article
export function getSlugFromPath(path: string): string {
	const parts = path.split('/');
	return parts[parts.length - 1];
}

// Build article URL from path
// e.g., /superbigshit/articles/tech/my-article -> /articles/tech/my-article
export function getArticleUrl(article: Article): string {
	return pathToUrl(article.path);
}

// Build category URL from category
// e.g., category with path /superbigshit/articles/tech -> /articles/tech
export function getCategoryUrl(category: Category): string {
	return pathToUrl(category.path);
}

// Build tag URL from tag path
// e.g., /superbigshit/tags/tech-stack/rust -> /settings/tags/tech-stack/rust
export function getTagUrl(tag: TagNode): string {
	const tagPath = tag.path.replace(TAGS_PATH, '');
	return `/settings/tags${tagPath}`;
}

// Create a RaisinReference from a TagNode
export function tagToReference(tag: TagNode, workspace = 'social'): RaisinReference {
	return {
		'raisin:ref': tag.id,
		'raisin:workspace': workspace,
		'raisin:path': tag.path
	};
}

// Extract tag name from reference path
// e.g., /superbigshit/tags/tech-stack/rust -> rust
export function getTagNameFromReference(ref: RaisinReference): string {
	if (!ref) return '';
	const parts = ref['raisin:path']?.split('/');
	if (!parts || parts.length === 0) return '';
	return parts[parts?.length - 1];
}

// Get parent path for a tag
// e.g., /superbigshit/tags/tech-stack/rust -> /superbigshit/tags/tech-stack
export function getTagParentPath(tagPath: string): string {
	const parts = tagPath.split('/');
	parts.pop();
	return parts.join('/');
}

// ============================================================================
// Article Connection Types (Graph/RELATE Integration)
// ============================================================================

// Relation types for article connections
export type ArticleRelationType =
	| 'continues'
	| 'updates'
	| 'corrects'
	| 'contradicts'
	| 'provides-evidence-for'
	| 'similar-to'
	| 'see-also';

// Metadata for each relation type (for UI display)
export const RELATION_TYPE_META: Record<
	ArticleRelationType,
	{
		label: string;
		description: string;
		color: string;
		icon: string;
	}
> = {
	continues: {
		label: 'Continues',
		description: 'This is a follow-up or sequel to the target article',
		color: '#3B82F6',
		icon: 'arrow-right'
	},
	updates: {
		label: 'Updates',
		description: 'This is a newer version or update of the target story',
		color: '#8B5CF6',
		icon: 'refresh-cw'
	},
	corrects: {
		label: 'Corrects',
		description: 'This article fixes or corrects information in the target',
		color: '#F59E0B',
		icon: 'pencil'
	},
	contradicts: {
		label: 'Contradicts',
		description: 'This article presents an opposing or conflicting view',
		color: '#EF4444',
		icon: 'x-circle'
	},
	'provides-evidence-for': {
		label: 'Provides Evidence',
		description: 'This article contains supporting data or sources',
		color: '#22C55E',
		icon: 'file-check'
	},
	'similar-to': {
		label: 'Similar To',
		description: 'This article covers related or similar content',
		color: '#6B7280',
		icon: 'link'
	},
	'see-also': {
		label: 'See Also',
		description: 'Editorial recommendation for further reading',
		color: '#6B7280',
		icon: 'bookmark'
	}
};

// Connection stored in article properties (outgoing)
export interface ArticleConnection {
	targetPath: string;
	targetId: string;
	targetTitle: string;
	relationType: ArticleRelationType;
	weight: number; // 0-100 in UI, stored as 0-1 in DB
	editorialNote?: string;
}

// Incoming connection (read-only display)
export interface IncomingConnection {
	sourcePath: string;
	sourceId: string;
	sourceTitle: string;
	relationType: ArticleRelationType;
	weight: number;
}

// Shared tag article (from GRAPH_TABLE 2-hop query)
export interface SharedTagArticle {
	articleId: string;
	articlePath: string;
	articleTitle: string;
	sharedTag: string;
	tagPath: string;
}

// Tag info from GRAPH_TABLE query
export interface ArticleTag {
	path: string;
	label: string;
	icon?: string;
	color?: string;
}

// Graph data for article display page
export interface ArticleGraphData {
	smartRelated: Array<{
		article: Article;
		weight: number;
		relationType: string;
	}>;
	timeline: {
		predecessors: Article[];
		successors: Article[];
	};
	correction: {
		title: string;
		path: string;
		publishedAt: string;
	} | null;
	// Article that THIS article corrects (outgoing corrects edge)
	correctsArticle: {
		title: string;
		path: string;
		publishedAt: string;
	} | null;
	opposingViews: Article[];
	evidence: Article[];
	// GRAPH_TABLE showcase: 2-hop pattern to find articles sharing same tags
	sharedTagArticles: SharedTagArticle[];
	// Article's tags via GRAPH_TABLE (cleaner than NEIGHBORS + JOIN)
	tags: ArticleTag[];
}

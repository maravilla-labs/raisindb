import { error } from '@sveltejs/kit';
import { queryMaybeUser, queryOneMaybeUser, executeMaybeUser } from '$lib/server/db';
import { ARTICLES_PATH, type Article, type ArticleGraphData, type ArticleTag, type Category, type SharedTagArticle } from '$lib/types';

function sanitizeLiteral(value: string): string {
	return value.replace(/'/g, "''");
}

export async function load({ params, parent, locals }) {
	const pathSegments = params.path?.split('/') || [];
	const fullDbPath = `${ARTICLES_PATH}/${params.path}`;
	const safePathLiteral = sanitizeLiteral(fullDbPath);
	const accessToken = locals.accessToken;

	// Get parent data (categories)
	const parentData = await parent();
	
	// First, check if this is an article (has more than 1 segment: category/slug)
	if (pathSegments.length >= 2) {
		// This could be an article - try to fetch it, include ANCESTOR to get category path
		const article = await queryOneMaybeUser<Article & { category_path: string }>(`
			SELECT id, path, name, node_type, properties, created_at, updated_at,
			       ANCESTOR(path, 3) AS category_path
			FROM social
			WHERE path = $1
			  AND node_type = 'news:Article'
		`, [fullDbPath], accessToken);

		if (article) {
			const categoryPath = article.category_path;

			// Increment view count
			await executeMaybeUser(`
				UPDATE social
				SET properties = jsonb_set(
					properties,
					'{views}',
					to_jsonb(COALESCE((properties ->> 'views')::INT, 0) + 1)
				)
				WHERE path = $1
			`, [fullDbPath], accessToken);

			// Get related articles using the category path from ANCESTOR
			// Only show published articles with publishing_date <= now
			const related = await queryMaybeUser<Article>(`
				SELECT id, path, name, node_type, properties, created_at, updated_at
				FROM social
				WHERE DESCENDANT_OF('${categoryPath}')
				  AND node_type = 'news:Article'
				  AND properties ->> 'status'::TEXT = 'published'
				  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
				  AND path != $1
				ORDER BY (properties ->> 'publishing_date')::TIMESTAMP DESC
				LIMIT 3
			`, [fullDbPath], accessToken);

			// ========================================
			// GRAPH DATA: All queries use GRAPH_TABLE for consistent SQL/PGQ syntax
			// ========================================

			// 1. Correction incoming edge - find articles that correct this one
			const correctionArticle = await queryOneMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this:Article)<-[:corrects]-(correction:Article)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						correction.id AS id,
						correction.path AS path,
						correction.name AS name,
						correction.node_type AS node_type,
						correction.properties AS properties,
						correction.created_at AS created_at,
						correction.updated_at AS updated_at
					)
				) AS g
				LIMIT 1
			`, [], accessToken);

			const correction = correctionArticle
				? {
					title: correctionArticle.properties?.title || correctionArticle.name || 'Correction',
					path: correctionArticle.path,
					publishedAt:
						correctionArticle.properties?.publishing_date || correctionArticle.updated_at || ''
				}
				: null;

			// 1b. Correction outgoing edge - find article that THIS article corrects
			const correctsArticleData = await queryOneMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this:Article)-[:corrects]->(original:Article)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						original.id AS id,
						original.path AS path,
						original.name AS name,
						original.node_type AS node_type,
						original.properties AS properties,
						original.created_at AS created_at,
						original.updated_at AS updated_at
					)
				) AS g
				LIMIT 1
			`, [], accessToken);

			const correctsArticle = correctsArticleData
				? {
					title: correctsArticleData.properties?.title || correctsArticleData.name || 'Original Article',
					path: correctsArticleData.path,
					publishedAt:
						correctsArticleData.properties?.publishing_date || correctsArticleData.updated_at || ''
				}
				: null;

			// 2. Timeline: predecessors - articles this one continues (multi-hop for full chain)
			const predecessors = await queryMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this)-[:continues*]->(prev)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						prev.id AS id,
						prev.path AS path,
						prev.name AS name,
						prev.node_type AS node_type,
						prev.properties AS properties,
						prev.created_at AS created_at,
						prev.updated_at AS updated_at
					)
				) AS g

				   ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC NULLS LAST, g.created_at ASC


			`, [], accessToken);

			// 3. Timeline: successors - articles that continue this one (multi-hop for full chain)
			const successors = await queryMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this:Article)<-[:continues*]-(next:Article)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						next.id AS id,
						next.path AS path,
						next.name AS name,
						next.node_type AS node_type,
						next.properties AS properties,
						next.created_at AS created_at,
						next.updated_at AS updated_at
					)
				) AS g
				ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC NULLS LAST, g.created_at ASC
			`, [], accessToken);

			// 4. Smart related - articles with similar-to, see-also, or updates relations
			const smartRelatedData = await queryMaybeUser<{
				id: string;
				path: string;
				relation_type: string;
				weight: number;
				properties: Article['properties'];
				name: string;
				node_type: string;
				created_at: string;
				updated_at: string;
			}>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this)-[r:\`similar-to\`|\`see-also\`|updates]->(related)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						related.id AS id,
						related.path AS path,
						related.name AS name,
						related.node_type AS node_type,
						related.properties AS properties,
						related.created_at AS created_at,
						related.updated_at AS updated_at,
						r.type AS relation_type,
						r.weight AS weight
					)
				) AS g
				ORDER BY g.weight DESC
				LIMIT 5
			`, [], accessToken);

			const smartRelated = smartRelatedData.map((r) => ({
				article: {
					id: r.id,
					path: r.path,
					name: r.name,
					node_type: r.node_type,
					properties: r.properties,
					created_at: r.created_at,
					updated_at: r.updated_at
				},
				weight: Math.round((r.weight ?? 0.75) * 100),
				relationType: r.relation_type
			}));

			// 5. Opposing views - articles that contradict this one (bidirectional)
			const opposingViews = await queryMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this:Article)-[:contradicts]-(other:Article)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						other.id AS id,
						other.path AS path,
						other.name AS name,
						other.node_type AS node_type,
						other.properties AS properties,
						other.created_at AS created_at,
						other.updated_at AS updated_at
					)
				) AS g
			`, [], accessToken);

			// 6. Evidence - articles providing evidence (bidirectional)
			const evidence = await queryMaybeUser<Article>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this:Article)-[:\`provides-evidence-for\`]-(other:Article)
					WHERE this.path = '${safePathLiteral}'
					COLUMNS (
						other.id AS id,
						other.path AS path,
						other.name AS name,
						other.node_type AS node_type,
						other.properties AS properties,
						other.created_at AS created_at,
						other.updated_at AS updated_at
					)
				) AS g
			`, [], accessToken);

			// 7. Articles sharing same tags (2-hop pattern)
			// Pattern: (thisArticle)-[:tagged-with]->(tag)<-[:tagged-with]-(otherArticle)
			const sharedTagArticles = await queryMaybeUser<{
				article_id: string;
				article_path: string;
				article_title: string;
				shared_tag: string;
				tag_path: string;
			}>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (this)-[:\`tagged-with\`]->(tag)<-[:\`tagged-with\`]-(other)
					WHERE this.path = '${safePathLiteral}'
					  AND other.path <> this.path
					COLUMNS (
						other.id AS article_id,
						other.path AS article_path,
						other.name AS article_title,
						tag.name AS shared_tag,
						tag.path AS tag_path
					)
				) AS g
				LIMIT 10
			`, [], accessToken);
			// Transform to typed array
			const sharedTagArticlesTyped: SharedTagArticle[] = sharedTagArticles.map(row => ({
				articleId: row.article_id,
				articlePath: row.article_path,
				articleTitle: row.article_title,
				sharedTag: row.shared_tag,
				tagPath: row.tag_path
			}));

			// 8. Get article's tags
			const articleTags = await queryMaybeUser<{
				path: string;
				label: string;
				icon: string | null;
				color: string | null;
			}>(`
				SELECT * FROM GRAPH_TABLE(
					MATCH (article:Article)-[:\`tagged-with\`]->(tag:Tag)
					WHERE article.path = '${safePathLiteral}'
					COLUMNS (
						tag.path AS path,
						tag.name AS label,
						tag.icon AS icon,
						tag.color AS color
					)
				) AS tags
			`, [], accessToken);

			const tags: ArticleTag[] = articleTags.map(t => ({
				path: t.path,
				label: t.label,
				icon: t.icon ?? undefined,
				color: t.color ?? undefined
			}));

			// Build graph data object
			const graphData: ArticleGraphData = {
				smartRelated,
				timeline: {
					predecessors,
					successors
				},
				correction,
				correctsArticle,
				opposingViews,
				evidence,
				sharedTagArticles: sharedTagArticlesTyped,
				tags
			};

			// Find the category from parent data
			const categorySlug = pathSegments[0];
			const category = parentData.categories.find((c: Category) => c.slug === categorySlug);

			return {
				type: 'article' as const,
				article,
				related,
				graphData,
				category,
				categories: parentData.categories,
				tags: parentData.tags
			};
		}
	}

	// Check if this is a category (single segment)
	if (pathSegments.length === 1) {
		const categorySlug = pathSegments[0];
		const categoryPath = `${ARTICLES_PATH}/${categorySlug}`;

		// Find the category from parent data
		const category = parentData.categories.find((c: Category) => c.slug === categorySlug);
		if (category) {
			// Get articles in this category
			// Only show published articles with publishing_date <= now
			const articles = await queryMaybeUser<Article>(`
				SELECT id, path, name, node_type, properties, created_at, updated_at
				FROM social
				WHERE CHILD_OF('${categoryPath}')
				  AND node_type = 'news:Article'
				  AND properties ->> 'status'::TEXT = 'published'
				  AND (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
				ORDER BY properties ->> 'publishing_date' DESC
				LIMIT 20
			`, [], accessToken);

			return {
				type: 'category' as const,
				category,
				articles,
				categories: parentData.categories,
				tags: parentData.tags
			};
		}
	}

	throw error(404, 'Page not found');
}

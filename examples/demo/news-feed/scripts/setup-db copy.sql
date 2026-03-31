-- Setup script for News Feed Demo
-- Run this against the RaisinDB instance to create the folder structure and sample data
--
-- This demo showcases:
-- 1. NodeType definitions with DDL
-- 2. Hierarchical tags using Reference properties
-- 3. Tree-structured content organization

-- ============================================================================
-- Schema: NodeType Definitions
-- ============================================================================

-- Drop existing types first (if they exist)
DROP NODETYPE 'news:Article';
DROP NODETYPE 'news:Tag';

-- Create news:Tag nodetype for hierarchical tagging
-- Tags can be nested (e.g., /tags/tech-stack/rust) and have visual properties
CREATE NODETYPE 'news:Tag' (
  PROPERTIES (
    label String REQUIRED LABEL 'Display Label' ORDER 1,
    icon String LABEL 'Lucide Icon Name' ORDER 2,
    color String LABEL 'Hex Color' ORDER 3
  )
  INDEXABLE
);

-- Create news:Article nodetype with both keywords (string array) and tags (references)
-- keywords: simple strings for fulltext search
-- tags: Reference array for structured relationships, queryable via REFERENCES()
-- publishing_date: when the article should be published (used for sorting and scheduling)
CREATE NODETYPE 'news:Article' (
  PROPERTIES (
    title String REQUIRED FULLTEXT LABEL 'Title' ORDER 1,
    slug String REQUIRED PROPERTY_INDEX LABEL 'URL Slug' ORDER 2,
    excerpt String LABEL 'Excerpt' ORDER 3,
    body String FULLTEXT LABEL 'Body Content' ORDER 4,
    category String PROPERTY_INDEX LABEL 'Category' ORDER 5,
    keywords Array OF String FULLTEXT LABEL 'Keywords' ORDER 6,
    tags Array OF Reference LABEL 'Tags' ORDER 7,
    featured Boolean DEFAULT false PROPERTY_INDEX LABEL 'Featured' ORDER 8,
    status String DEFAULT 'draft' PROPERTY_INDEX LABEL 'Status' ORDER 9,
    publishing_date Date PROPERTY_INDEX LABEL 'Publishing Date' ORDER 10,
    views Number DEFAULT 0 LABEL 'View Count' ORDER 11,
    author String PROPERTY_INDEX LABEL 'Author' ORDER 12,
    imageUrl String LABEL 'Image URL' ORDER 13
  )
  COMPOUND_INDEX 'idx_article_status_publishing_date' ON (
    __node_type,
    status,
    publishing_date DESC
  )
  PUBLISHABLE
  INDEXABLE
);

-- ============================================================================
-- Data: Clean up and create content
-- ============================================================================

-- Clean up existing data first
DELETE FROM social WHERE node_type = 'news:Article' OR node_type = 'news:Tag' OR path LIKE '/superbigshit/%';

-- Create root folder
UPSERT INTO social (path, node_type, name) VALUES ('/superbigshit', 'raisin:Folder', 'Demo News');
UPSERT INTO social (path, node_type, name) VALUES ('/superbigshit/articles', 'raisin:Folder', 'Articles');

-- ============================================================================
-- Tags: Hierarchical tag structure
-- ============================================================================

-- Create tags folder
UPSERT INTO social (path, node_type, name) VALUES ('/superbigshit/tags', 'raisin:Folder', 'Tags');

-- Top-level tag categories
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/topics', 'news:Tag', 'Topics', '{"label": "Topics", "icon": "folder", "color": "#6B7280"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack', 'news:Tag', 'Tech Stack', '{"label": "Tech Stack", "icon": "code", "color": "#3B82F6"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/audience', 'news:Tag', 'Audience', '{"label": "Audience", "icon": "users", "color": "#10B981"}'::JSONB);

-- Nested tags under Topics
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/topics/trending', 'news:Tag', 'Trending', '{"label": "Trending", "icon": "trending-up", "color": "#EF4444"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/topics/breaking', 'news:Tag', 'Breaking News', '{"label": "Breaking News", "icon": "zap", "color": "#F59E0B"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/topics/analysis', 'news:Tag', 'Analysis', '{"label": "Analysis", "icon": "bar-chart-2", "color": "#8B5CF6"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/topics/opinion', 'news:Tag', 'Opinion', '{"label": "Opinion", "icon": "message-square", "color": "#06B6D4"}'::JSONB);

-- Nested tags under Tech Stack
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack/rust', 'news:Tag', 'Rust', '{"label": "Rust", "icon": "cog", "color": "#DEA584"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack/typescript', 'news:Tag', 'TypeScript', '{"label": "TypeScript", "icon": "file-code", "color": "#3178C6"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack/ai', 'news:Tag', 'AI/ML', '{"label": "AI/ML", "icon": "brain", "color": "#EC4899"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack/web', 'news:Tag', 'Web Dev', '{"label": "Web Dev", "icon": "globe", "color": "#14B8A6"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/tech-stack/databases', 'news:Tag', 'Databases', '{"label": "Databases", "icon": "database", "color": "#F97316"}'::JSONB);

-- Nested tags under Audience
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/audience/beginners', 'news:Tag', 'Beginners', '{"label": "Beginners", "icon": "book-open", "color": "#22C55E"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/audience/advanced', 'news:Tag', 'Advanced', '{"label": "Advanced", "icon": "graduation-cap", "color": "#6366F1"}'::JSONB);
UPSERT INTO social (path, node_type, name, properties) VALUES
  ('/superbigshit/tags/audience/enterprise', 'news:Tag', 'Enterprise', '{"label": "Enterprise", "icon": "building", "color": "#78716C"}'::JSONB);

-- ============================================================================
-- Categories: Article category folders
-- ============================================================================

-- Create category folders with properties (color, label, order for navigation)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech',
  'raisin:Folder',
  'Technology',
  '{"label": "Technology", "color": "#3B82F6", "order": 1}'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business',
  'raisin:Folder',
  'Business',
  '{"label": "Business", "color": "#10B981", "order": 2}'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports',
  'raisin:Folder',
  'Sports',
  '{"label": "Sports", "color": "#F59E0B", "order": 3}'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/entertainment',
  'raisin:Folder',
  'Entertainment',
  '{"label": "Entertainment", "color": "#8B5CF6", "order": 4}'::JSONB
);

-- ============================================================================
-- Articles: Sample content with keywords (string[]) and tags (Reference[])
-- keywords: simple strings for fulltext search
-- tags: Reference objects queryable via REFERENCES('workspace:/path')
-- ============================================================================

-- Tech Articles
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/rust-web-development-2025',
  'news:Article',
  'The Rise of Rust in Web Development',
  '{
    "title": "The Rise of Rust in Web Development",
    "slug": "rust-web-development-2025",
    "excerpt": "Discover why Rust is becoming the go-to language for building high-performance, memory-safe web applications.",
    "body": "# The Rise of Rust in Web Development\n\nRust has been steadily gaining popularity in the web development community, and for good reason.\n\n## Why Rust?\n\n### Memory Safety\nRust ownership system ensures memory safety without a garbage collector.\n\n### Performance\nWith zero-cost abstractions and no runtime overhead, Rust applications can match or exceed the performance of C and C++.\n\n### Growing Ecosystem\nFrameworks like Actix-web, Axum, and Rocket make building web services a joy.\n\nThe future of web development is here.",
    "keywords": ["rust", "web development", "programming", "performance"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/rust", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"}
    ],
    "featured": true,
    "status": "published",
    "publishing_date": "2025-11-28T10:00:00Z",
    "views": 1247,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1555066931-4365d14bab8c?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-coding-assistants',
  'news:Article',
  'AI Coding Assistants Are Changing How We Write Software',
  '{
    "title": "AI Coding Assistants Are Changing How We Write Software",
    "slug": "ai-coding-assistants",
    "excerpt": "From GitHub Copilot to Claude, AI-powered coding tools are revolutionizing software development workflows.",
    "body": "# AI Coding Assistants Are Changing How We Write Software\n\nThe landscape of software development is undergoing a fundamental transformation.\n\n## The New Development Workflow\n\nDevelopers are increasingly working alongside AI tools that can:\n\n- Generate boilerplate code in seconds\n- Suggest optimizations based on best practices\n- Debug issues by analyzing error messages\n- Write documentation automatically\n\n## Impact on Productivity\n\nStudies show that developers using AI assistants can be 30-50% more productive on routine tasks.",
    "keywords": ["AI", "coding", "productivity", "GitHub Copilot"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-25T14:30:00Z",
    "views": 892,
    "author": "Marcus Johnson",
    "imageUrl": "https://images.unsplash.com/photo-1677442136019-21780ecad995?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/typescript-5-features',
  'news:Article',
  'TypeScript 5.x Features That Will Change Your Codebase',
  '{
    "title": "TypeScript 5.x Features That Will Change Your Codebase",
    "slug": "typescript-5-features",
    "excerpt": "Explore the latest TypeScript features including decorators, const type parameters, and more.",
    "body": "# TypeScript 5.x Features\n\nTypeScript continues to evolve with powerful new features.\n\n## Key Features\n\n### Decorators (Stage 3)\nDecorators are now a stable language feature.\n\n### Const Type Parameters\nCreate more precise types with const generics.\n\n### satisfies Operator\nValidate types without changing the inferred type.\n\n## Migration Tips\n\nUpgrading to TypeScript 5.x is straightforward for most projects.",
    "keywords": ["TypeScript", "JavaScript", "programming"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/typescript", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/advanced", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-20T09:00:00Z",
    "views": 654,
    "author": "Elena Rodriguez",
    "imageUrl": "https://images.unsplash.com/photo-1516116216624-53e697fedbea?w=800"
  }'::JSONB
);

-- Business Articles
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/startup-funding-guide',
  'news:Article',
  'The Complete Guide to Startup Funding in 2025',
  '{
    "title": "The Complete Guide to Startup Funding in 2025",
    "slug": "startup-funding-guide",
    "excerpt": "Navigate the fundraising landscape with insights from successful founders and investors.",
    "body": "# The Complete Guide to Startup Funding in 2025\n\nRaising capital for your startup has evolved significantly.\n\n## Funding Stages\n\n### Pre-Seed ($50K - $500K)\n- Friends and family\n- Angel investors\n- Accelerators\n\n### Seed ($500K - $3M)\n- Seed-stage VCs\n- Micro VCs\n\n### Series A ($3M - $15M)\n- Traditional VCs\n- Growth equity\n\n## What Investors Look For\n\n1. Strong founding team\n2. Large addressable market\n3. Product-market fit\n4. Defensible moat",
    "keywords": ["startups", "funding", "venture capital", "entrepreneurship"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"}
    ],
    "featured": true,
    "status": "published",
    "publishing_date": "2025-11-29T08:00:00Z",
    "views": 2341,
    "author": "David Kim",
    "imageUrl": "https://images.unsplash.com/photo-1460925895917-afdab827c52f?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/remote-work-trends',
  'news:Article',
  'Remote Work in 2025: Trends Shaping the Future',
  '{
    "title": "Remote Work in 2025: Trends Shaping the Future",
    "slug": "remote-work-trends",
    "excerpt": "How companies are adapting their strategies for the new era of distributed work.",
    "body": "# Remote Work in 2025\n\nThe workplace has fundamentally changed.\n\n## Key Trends\n\n### Hybrid-First Models\nMost companies have settled on hybrid arrangements, with 2-3 days in office becoming the norm.\n\n### Asynchronous Communication\nTools and processes that support async work are crucial for global teams.\n\n### Virtual Collaboration Spaces\nBeyond video calls, companies are investing in virtual offices.\n\n## Challenges\n\n- Maintaining company culture\n- Preventing burnout\n- Ensuring equity between remote and in-office workers",
    "keywords": ["remote work", "hybrid", "future of work"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-15T11:00:00Z",
    "views": 1156,
    "author": "Amanda Foster",
    "imageUrl": "https://images.unsplash.com/photo-1522202176988-66273c2fd55f?w=800"
  }'::JSONB
);

-- Sports Articles
-- Note: This article has a future publishing_date - it will be hidden from listings until that date
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/olympic-preview-2028',
  'news:Article',
  'LA 2028 Olympics: What to Expect',
  '{
    "title": "LA 2028 Olympics: What to Expect",
    "slug": "olympic-preview-2028",
    "excerpt": "A comprehensive preview of the upcoming Summer Olympics in Los Angeles.",
    "body": "# LA 2028 Olympics: What to Expect\n\nLos Angeles is gearing up to host its third Summer Olympics.\n\n## New Sports\n\nThe 2028 Games will feature several new additions:\n- Cricket (returning after over a century)\n- Flag football\n- Squash\n- Lacrosse\n\n## Venues\n\nLA is leveraging its existing world-class venues:\n- SoFi Stadium - Opening and closing ceremonies\n- Crypto.com Arena - Basketball and gymnastics\n- Rose Bowl - Soccer finals\n\n## Sustainability Goals\n\nThe organizers have committed to the most sustainable Olympics ever.",
    "keywords": ["Olympics", "LA 2028", "sports"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/breaking", "raisin:workspace": "social"}
    ],
    "featured": true,
    "status": "published",
    "publishing_date": "2025-12-05T12:00:00Z",
    "views": 1893,
    "author": "Michael Torres",
    "imageUrl": "https://encrypted-tbn0.gstatic.com/images?q=tbn:ANd9GcQMBF1cnYE7RwmXhvv9gLncW0SeMogfMayQlw&s"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/esports-mainstream',
  'news:Article',
  'Esports Goes Mainstream: Record Viewership and Investments',
  '{
    "title": "Esports Goes Mainstream: Record Viewership and Investments",
    "slug": "esports-mainstream",
    "excerpt": "Competitive gaming reaches new heights with billion-dollar tournaments and mainstream recognition.",
    "body": "# Esports Goes Mainstream\n\nCompetitive gaming has officially entered the mainstream.\n\n## By the Numbers\n\n- 500M+ global esports audience\n- $1.8B industry revenue\n- 100K+ professional players worldwide\n\n## Top Games\n\n1. League of Legends\n2. Valorant\n3. Counter-Strike 2\n4. Dota 2\n5. Fortnite\n\n## The Rise of Amateur Leagues\n\nCollegiate esports programs have exploded, with over 200 universities offering scholarships.",
    "keywords": ["esports", "gaming", "competition"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-10T16:00:00Z",
    "views": 789,
    "author": "Jason Park",
    "imageUrl": "https://images.unsplash.com/photo-1542751371-adc38448a05e?w=800"
  }'::JSONB
);

-- Entertainment Articles
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/entertainment/streaming-wars-2025',
  'news:Article',
  'The Streaming Wars: Who is Winning in 2025?',
  '{
    "title": "The Streaming Wars: Who is Winning in 2025?",
    "slug": "streaming-wars-2025",
    "excerpt": "An analysis of the streaming landscape as platforms compete for viewers and content.",
    "body": "# The Streaming Wars: Who is Winning in 2025?\n\nThe battle for streaming dominance continues to evolve.\n\n## Market Share\n\n- Netflix: 280M subscribers\n- Disney+: 175M subscribers\n- Amazon Prime: 200M subscribers\n- Max: 95M subscribers\n\n## Key Trends\n\n### Ad-Supported Tiers\nEvery major platform now offers cheaper ad-supported options.\n\n### Live Sports\nStreaming rights for major sports leagues are reshaping the competitive landscape.\n\n## The Future\n\nExpect more consolidation and bundling as platforms seek profitability.",
    "keywords": ["streaming", "Netflix", "Disney+", "media"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-22T15:00:00Z",
    "views": 1567,
    "author": "Rachel Green",
    "imageUrl": "https://images.unsplash.com/photo-1522869635100-9f4c5e86aa37?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/entertainment/music-ai-revolution',
  'news:Article',
  'How AI is Transforming Music Creation and Distribution',
  '{
    "title": "How AI is Transforming Music Creation and Distribution",
    "slug": "music-ai-revolution",
    "excerpt": "From AI-generated compositions to personalized playlists, technology is reshaping the music industry.",
    "body": "# How AI is Transforming Music Creation\n\nArtificial intelligence is revolutionizing every aspect of the music industry.\n\n## AI in Music Creation\n\n### Composition Assistance\nArtists are using AI tools to:\n- Generate chord progressions\n- Create backing tracks\n- Experiment with new sounds\n\n### Vocal Synthesis\nAI can now clone voices with stunning accuracy.\n\n## Distribution and Discovery\n\n### Personalized Recommendations\nStreaming platforms use AI to create tailored playlists.\n\n### Trend Prediction\nLabels use AI to identify emerging artists and predict hit potential.",
    "keywords": ["music", "AI", "technology", "streaming"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-01T13:00:00Z",
    "views": 934,
    "author": "Chris Williams",
    "imageUrl": "https://images.unsplash.com/photo-1511379938547-c1f69419868d?w=800"
  }'::JSONB
);

-- ============================================================================
-- EXTENDED ARTICLES FOR GRAPH SHOWCASE
-- These articles create a rich network of semantic connections
-- ============================================================================

-- ============================================================================
-- Theme 1: AI Revolution Series (Story Chain)
-- ============================================================================

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-revolution-part-1',
  'news:Article',
  'The AI Revolution Begins: How Machine Learning is Changing Everything',
  '{
    "title": "The AI Revolution Begins: How Machine Learning is Changing Everything",
    "slug": "ai-revolution-part-1",
    "excerpt": "Part 1 of our comprehensive series on the AI transformation sweeping across industries.",
    "body": "# The AI Revolution Begins\n\nIn 2023, artificial intelligence crossed a threshold that many thought was decades away. Large language models demonstrated capabilities that surprised even their creators.\n\n## The Breakthrough Moment\n\nThe release of advanced AI systems marked a turning point. These systems could:\n\n- Generate human-quality text\n- Understand complex contexts\n- Solve problems creatively\n\n## Industry Impact\n\nEvery sector began feeling the effects:\n\n### Healthcare\nDiagnostic AI systems are now matching specialist accuracy.\n\n### Finance\nTrading algorithms have become more sophisticated.\n\n### Education\nPersonalized learning is finally becoming reality.\n\n*This is Part 1 of our 4-part series on the AI Revolution.*",
    "keywords": ["AI", "machine learning", "technology", "series", "revolution"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"}
    ],
    "featured": true,
    "status": "published",
    "publishing_date": "2025-10-01T10:00:00Z",
    "views": 8420,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1677442136019-21780ecad995?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-revolution-part-2',
  'news:Article',
  'AI Revolution Part 2: The Enterprise Response',
  '{
    "title": "AI Revolution Part 2: How Enterprises Are Adapting",
    "slug": "ai-revolution-part-2",
    "excerpt": "Part 2: Major corporations restructure operations around AI capabilities.",
    "body": "# The Enterprise Response\n\nFollowing the initial AI boom, enterprises have scrambled to adapt their operations and strategies.\n\n## Corporate AI Adoption\n\n### Infrastructure Changes\nCompanies are investing billions in:\n- GPU clusters\n- Data pipelines\n- AI talent acquisition\n\n### Organizational Restructuring\nNew roles are emerging:\n- Chief AI Officer\n- AI Ethics Lead\n- Prompt Engineers\n\n## Case Studies\n\n### Microsoft\nIntegrated AI across all products with Copilot.\n\n### Google\nRestructured around AI-first initiatives.\n\n### Amazon\nExpanded AWS AI services dramatically.\n\n*Continued in Part 3: The Human Factor*",
    "keywords": ["AI", "enterprise", "digital transformation", "corporate"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-15T10:00:00Z",
    "views": 5890,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1485827404703-89b55fcc595e?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-revolution-part-3',
  'news:Article',
  'AI Revolution Part 3: The Human Factor',
  '{
    "title": "AI Revolution Part 3: What It Means for Workers",
    "slug": "ai-revolution-part-3",
    "excerpt": "Part 3: The human impact of AI transformation and workforce adaptation strategies.",
    "body": "# The Human Factor\n\nAs AI continues its march through industries, workers face new realities and opportunities.\n\n## Job Market Transformation\n\n### Jobs at Risk\n- Routine data entry\n- Basic customer service\n- Simple content creation\n\n### Emerging Opportunities\n- AI trainers and supervisors\n- Ethics consultants\n- Human-AI collaboration specialists\n\n## Reskilling Imperative\n\nWorkers must adapt by:\n\n1. Learning to work alongside AI\n2. Developing uniquely human skills\n3. Embracing continuous learning\n\n## Government Response\n\nPolicies are being developed for:\n- Universal Basic Income trials\n- Reskilling programs\n- AI regulation frameworks\n\n*Final part coming: The Future Landscape*",
    "keywords": ["AI", "jobs", "workforce", "future of work", "reskilling"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-01T10:00:00Z",
    "views": 4156,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1531746790731-6c087fecd65a?w=800"
  }'::JSONB
);

UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-revolution-part-4',
  'news:Article',
  'AI Revolution Part 4: The Future Landscape',
  '{
    "title": "AI Revolution Part 4: What Comes Next",
    "slug": "ai-revolution-part-4",
    "excerpt": "The conclusion of our series: Predictions and possibilities for AI in the next decade.",
    "body": "# The Future Landscape\n\nIn this final installment, we explore what the next decade might bring.\n\n## Near-Term Predictions (2025-2027)\n\n- AGI remains elusive but capabilities grow\n- Multimodal AI becomes standard\n- AI regulation matures globally\n\n## Medium-Term (2028-2030)\n\n- AI assistants become truly personal\n- Scientific discovery accelerates\n- Creative industries transformed\n\n## Long-Term Possibilities\n\n### Optimistic Scenario\nAI solves major challenges: climate, disease, poverty.\n\n### Cautious Scenario\nManaged integration with strong governance.\n\n### Concerning Scenario\nConcentration of power, job displacement.\n\n## Conclusion\n\nThe AI revolution is not a future event—it is happening now. How we navigate it will define the coming decades.\n\n*Thank you for following this 4-part series.*",
    "keywords": ["AI", "future", "predictions", "AGI", "technology"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"}
    ],
    "featured": true,
    "status": "published",
    "publishing_date": "2025-11-15T10:00:00Z",
    "views": 6234,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1620712943543-bcc4688e7485?w=800"
  }'::JSONB
);

-- AI Skeptic's View (Contradicts AI Revolution)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-skeptics-view',
  'news:Article',
  'Is AI Overhyped? A Critical Perspective on the Revolution',
  '{
    "title": "Is AI Overhyped? A Critical Perspective on the Revolution",
    "slug": "ai-skeptics-view",
    "excerpt": "Not everyone is convinced by AI promises. Here is the case for healthy skepticism.",
    "body": "# A Critical Look at AI Hype\n\nWhile tech optimists celebrate, many experts urge caution about AI capabilities and timelines.\n\n## The Hype Problem\n\n### Overpromised, Underdelivered\n- Self-driving cars still not mainstream\n- AGI timeline constantly pushed back\n- Many AI products are glorified autocomplete\n\n### The Reality Check\n\n1. **Hallucinations remain unsolved** - AI confidently makes up facts\n2. **Energy costs are staggering** - Environmental impact ignored\n3. **Bias persists** - Training data reflects societal problems\n\n## Historical Precedent\n\nWeve seen this before:\n- AI Winter 1.0 (1974-1980)\n- AI Winter 2.0 (1987-1993)\n- Will there be an AI Winter 3.0?\n\n## Balanced Assessment\n\nAI is a powerful tool, not magic. We should:\n- Set realistic expectations\n- Invest in safety research\n- Question vendor claims critically",
    "keywords": ["AI", "criticism", "skepticism", "hype", "technology"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-20T14:00:00Z",
    "views": 7521,
    "author": "Dr. James Mitchell",
    "imageUrl": "https://images.unsplash.com/photo-1504868584819-f8e8b4b6d7e3?w=800"
  }'::JSONB
);

-- AI Productivity Correction (Corrects AI Coding Assistants article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-productivity-correction',
  'news:Article',
  'Correction: Updated Data on AI Coding Assistant Productivity',
  '{
    "title": "Correction: Updated Data on AI Coding Assistant Productivity",
    "slug": "ai-productivity-correction",
    "excerpt": "New research provides more accurate productivity figures for AI coding tools.",
    "body": "# Correction Notice\n\nOur previous article on AI Coding Assistants cited productivity gains of 30-50%. Updated research from Stanford and MIT provides more nuanced data.\n\n## Updated Findings\n\n### Actual Productivity Gains\n- **Junior developers**: 25-35% improvement\n- **Senior developers**: 10-15% improvement\n- **Complex tasks**: Minimal or negative impact\n\n### Methodology Issues\n\nThe original studies had limitations:\n1. Self-selected participants\n2. Simple task focus\n3. Short-term measurement\n\n## What This Means\n\nAI coding assistants are valuable but:\n- Best for boilerplate code\n- Less helpful for architecture decisions\n- Require verification overhead\n\n## Our Commitment\n\nWe strive for accuracy and will update our coverage as new data emerges.",
    "keywords": ["AI", "correction", "productivity", "research", "coding"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/breaking", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-27T09:00:00Z",
    "views": 2234,
    "author": "Marcus Johnson",
    "imageUrl": "https://images.unsplash.com/photo-1504868584819-f8e8b4b6d7e3?w=800"
  }'::JSONB
);

-- AI Ethics Data (Provides Evidence for AI Revolution)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-ethics-data',
  'news:Article',
  'AI Ethics Survey 2025: What the Data Reveals',
  '{
    "title": "AI Ethics Survey 2025: What the Data Reveals",
    "slug": "ai-ethics-data",
    "excerpt": "Comprehensive survey data on AI ethics concerns across 50 countries.",
    "body": "# AI Ethics Survey 2025\n\nWe surveyed 50,000 professionals across 50 countries about AI ethics concerns.\n\n## Key Findings\n\n### Top Concerns\n1. **Job displacement** (78% worried)\n2. **Privacy violations** (72%)\n3. **Algorithmic bias** (68%)\n4. **Autonomous weapons** (65%)\n\n### Regional Differences\n\n| Region | Top Concern |\n|--------|-------------|\n| North America | Privacy |\n| Europe | Regulation |\n| Asia | Job displacement |\n\n## Demographics\n\n### By Age Group\n- 18-30: More optimistic\n- 31-50: Mixed views\n- 51+: More concerned\n\n### By Industry\n- Tech workers: 60% optimistic\n- Healthcare: 55% cautious\n- Education: 70% concerned\n\n## Full Dataset\n\nComplete data available for download at our research portal.",
    "keywords": ["AI", "ethics", "survey", "data", "research"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-05T11:00:00Z",
    "views": 3456,
    "author": "Dr. Emily Watson",
    "imageUrl": "https://images.unsplash.com/photo-1551288049-bebda4e38f71?w=800"
  }'::JSONB
);

-- AI in Healthcare (Similar to AI Revolution)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/ai-in-healthcare',
  'news:Article',
  'AI in Healthcare: From Diagnosis to Drug Discovery',
  '{
    "title": "AI in Healthcare: From Diagnosis to Drug Discovery",
    "slug": "ai-in-healthcare",
    "excerpt": "How artificial intelligence is revolutionizing medicine and patient care.",
    "body": "# AI in Healthcare\n\nThe healthcare industry is experiencing an AI transformation unlike any other sector.\n\n## Diagnostic Applications\n\n### Radiology\n- AI reads X-rays with 95% accuracy\n- Early cancer detection improved by 30%\n- Reduced radiologist workload\n\n### Pathology\n- Automated slide analysis\n- Rare disease identification\n- Quality assurance\n\n## Drug Discovery\n\n### Accelerated Development\n- AlphaFold revolutionized protein structure prediction\n- Drug candidates identified in weeks vs years\n- Reduced R&D costs by billions\n\n## Patient Care\n\n### Personalized Medicine\n- Treatment plans tailored to genetics\n- Dosage optimization\n- Side effect prediction\n\n## Challenges\n\n- Regulatory approval processes\n- Data privacy concerns\n- Integration with existing systems",
    "keywords": ["AI", "healthcare", "medicine", "diagnosis", "drug discovery"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-25T09:00:00Z",
    "views": 4123,
    "author": "Dr. Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1576091160399-112ba8d25d1d?w=800"
  }'::JSONB
);

-- ============================================================================
-- Theme 2: Startup & Business
-- ============================================================================

-- Startup Funding Part 2 (Continues existing guide)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/startup-funding-part-2',
  'news:Article',
  'Startup Funding Guide Part 2: Mastering the Pitch',
  '{
    "title": "Startup Funding Guide Part 2: Mastering the Pitch",
    "slug": "startup-funding-part-2",
    "excerpt": "Part 2: How to create a compelling pitch deck and nail your investor meetings.",
    "body": "# Mastering the Pitch\n\nFollowing our overview of funding stages, lets dive into pitch mastery.\n\n## The Perfect Pitch Deck\n\n### Essential Slides\n1. **Problem** (1 slide)\n2. **Solution** (1-2 slides)\n3. **Market Size** (1 slide)\n4. **Business Model** (1 slide)\n5. **Traction** (1-2 slides)\n6. **Team** (1 slide)\n7. **Ask** (1 slide)\n\n## Common Mistakes\n\n- Too many slides (keep it under 15)\n- No clear ask\n- Unrealistic projections\n- Ignoring competition\n\n## The Meeting\n\n### Before\n- Research the investor\n- Practice your story\n- Prepare for tough questions\n\n### During\n- Be concise\n- Show passion\n- Listen actively\n\n### After\n- Follow up within 24 hours\n- Provide requested materials\n- Update on progress",
    "keywords": ["startups", "funding", "pitch", "investors", "venture capital"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-30T08:00:00Z",
    "views": 1890,
    "author": "David Kim",
    "imageUrl": "https://images.unsplash.com/photo-1553877522-43269d4ea984?w=800"
  }'::JSONB
);

-- VC Investment Data (Provides Evidence for Funding Guide)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/vc-investment-q3-data',
  'news:Article',
  'Q3 2025: Record-Breaking AI Venture Capital Investments',
  '{
    "title": "Q3 2025: Record-Breaking AI Venture Capital Investments",
    "slug": "vc-investment-q3-data",
    "excerpt": "Data analysis reveals unprecedented capital flow into AI startups this quarter.",
    "body": "# Record AI Investment Quarter\n\nVC firms deployed $45B into AI startups in Q3 2025, shattering previous records.\n\n## By the Numbers\n\n| Metric | Q3 2025 | Q3 2024 | Change |\n|--------|---------|---------|--------|\n| Total Invested | $45B | $28B | +61% |\n| Deal Count | 1,247 | 892 | +40% |\n| Avg Deal Size | $36M | $31M | +16% |\n\n## Top Sectors\n\n1. **Enterprise AI** - $15B\n2. **Healthcare AI** - $8B\n3. **Autonomous Systems** - $7B\n4. **AI Infrastructure** - $6B\n\n## Geographic Distribution\n\n- USA: 58%\n- Europe: 22%\n- Asia: 18%\n- Other: 2%\n\n## Notable Deals\n\n- Anthropic: $4B Series E\n- Mistral: $2B Series B\n- Cohere: $1.5B Series D\n\n## Methodology\n\nData sourced from PitchBook, Crunchbase, and proprietary surveys.",
    "keywords": ["venture capital", "AI", "investment", "data", "startups"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-05T11:00:00Z",
    "views": 5456,
    "author": "David Kim",
    "imageUrl": "https://images.unsplash.com/photo-1551288049-bebda4e38f71?w=800"
  }'::JSONB
);

-- Bootstrapping vs VC (Contradicts Funding Guide)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/bootstrapping-vs-vc',
  'news:Article',
  'Why Bootstrapping Beats VC: The Case Against Fundraising',
  '{
    "title": "Why Bootstrapping Beats VC: The Case Against Fundraising",
    "slug": "bootstrapping-vs-vc",
    "excerpt": "Venture capital isnt always the answer. Heres why self-funding might be better.",
    "body": "# The Case for Bootstrapping\n\nWhile VC funding dominates headlines, many successful companies chose a different path.\n\n## The VC Trap\n\n### What They Dont Tell You\n- Founder dilution can exceed 80%\n- Board control often lost by Series B\n- Pressure for 10x returns distorts decisions\n\n### The Statistics\n- 90% of VC-backed startups fail\n- Of successes, founders average <20% ownership\n- Only 1% achieve unicorn status\n\n## Bootstrapping Success Stories\n\n### Mailchimp\n- Bootstrapped for 20 years\n- Sold for $12B\n- Founders kept majority stake\n\n### Basecamp\n- Profitable from year one\n- 20+ years independent\n- Team of 50, millions in revenue\n\n## When Bootstrapping Works\n\n1. **B2B SaaS** with recurring revenue\n2. **Consulting-to-product** transitions\n3. **Niche markets** with loyal customers\n\n## The Hybrid Approach\n\nConsider: Bootstrap to profitability, then raise strategically if needed.",
    "keywords": ["bootstrapping", "startups", "VC", "funding", "entrepreneurship"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/advanced", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-05T14:00:00Z",
    "views": 3892,
    "author": "Jason Fried",
    "imageUrl": "https://images.unsplash.com/photo-1579621970563-ebec7560ff3e?w=800"
  }'::JSONB
);

-- Remote Work Study (Provides Evidence for Remote Work article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/remote-work-study',
  'news:Article',
  '2025 Remote Work Study: Productivity and Satisfaction Data',
  '{
    "title": "2025 Remote Work Study: Productivity and Satisfaction Data",
    "slug": "remote-work-study",
    "excerpt": "Comprehensive study of 10,000 remote workers reveals surprising insights.",
    "body": "# 2025 Remote Work Study\n\nWe surveyed 10,000 workers across 500 companies to understand remote work realities.\n\n## Key Findings\n\n### Productivity\n- **Remote workers**: 13% more productive on average\n- **Hybrid workers**: 9% more productive\n- **Office-only**: Baseline\n\n### Satisfaction Scores\n| Work Mode | Satisfaction |\n|-----------|-------------|\n| Fully Remote | 8.2/10 |\n| Hybrid 2-3 days | 7.8/10 |\n| Office Only | 6.4/10 |\n\n## Challenges Identified\n\n1. **Loneliness** - 45% report feeling isolated\n2. **Career growth** - 38% worry about visibility\n3. **Work-life blur** - 52% work longer hours\n\n## Best Practices\n\n### From High-Performing Remote Teams\n- Async-first communication\n- Regular video check-ins\n- Virtual social events\n- Clear documentation\n\n## Methodology\n\nDouble-blind survey, statistical significance p<0.05.",
    "keywords": ["remote work", "productivity", "study", "data", "hybrid"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-10T11:00:00Z",
    "views": 2156,
    "author": "Amanda Foster",
    "imageUrl": "https://images.unsplash.com/photo-1522202176988-66273c2fd55f?w=800"
  }'::JSONB
);

-- Startup Failures Analysis (See-also for Funding Guide)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/startup-failures',
  'news:Article',
  'Why Startups Fail: Analysis of 200 Post-Mortems',
  '{
    "title": "Why Startups Fail: Analysis of 200 Post-Mortems",
    "slug": "startup-failures",
    "excerpt": "We analyzed 200 startup post-mortems to identify the most common failure patterns.",
    "body": "# Why Startups Fail\n\nAnalyzing 200 startup post-mortems reveals consistent patterns of failure.\n\n## Top Failure Reasons\n\n### 1. No Market Need (42%)\nThe most common killer. Signs:\n- Inability to achieve product-market fit\n- Low user engagement\n- High churn rates\n\n### 2. Ran Out of Cash (29%)\n- Poor financial planning\n- Overestimated runway\n- Failed fundraising rounds\n\n### 3. Wrong Team (23%)\n- Founder conflicts\n- Key departures\n- Skill gaps\n\n### 4. Got Outcompeted (19%)\n- Larger players entered market\n- Better-funded competitors\n- Couldnt differentiate\n\n## Survival Strategies\n\n1. **Validate before building** - Talk to 100 customers\n2. **Keep runway long** - 18+ months minimum\n3. **Hire slow, fire fast** - Culture fit matters\n4. **Focus ruthlessly** - Do one thing well\n\n## Case Studies\n\nDetailed analysis of 10 notable failures included in appendix.",
    "keywords": ["startups", "failure", "analysis", "entrepreneurship", "lessons"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-10-28T09:00:00Z",
    "views": 6789,
    "author": "David Kim",
    "imageUrl": "https://images.unsplash.com/photo-1523474253046-8cd2748b5fd2?w=800"
  }'::JSONB
);

-- Future of Work & AI (Cross-category)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/business/future-work-ai',
  'news:Article',
  'The Future of Work: How AI is Reshaping Every Job',
  '{
    "title": "The Future of Work: How AI is Reshaping Every Job",
    "slug": "future-work-ai",
    "excerpt": "From executives to entry-level, no role is immune to AI transformation.",
    "body": "# The Future of Work\n\nEvery job will be touched by AI. Heres what to expect.\n\n## Jobs Most Affected\n\n### High Impact\n- Data entry and processing\n- Customer service (basic)\n- Content moderation\n- Paralegal work\n\n### Medium Impact\n- Software development\n- Marketing and advertising\n- Financial analysis\n- Medical diagnosis\n\n### Lower Impact\n- Trades and crafts\n- Healthcare (physical)\n- Leadership roles\n- Creative direction\n\n## New Skills Required\n\n1. **AI literacy** - Understanding capabilities and limits\n2. **Prompt engineering** - Communicating with AI systems\n3. **Critical thinking** - Verifying AI outputs\n4. **Adaptability** - Continuous learning mindset\n\n## Company Preparations\n\n- Reskilling programs\n- Role redesign initiatives\n- Ethics frameworks\n- Change management",
    "keywords": ["future of work", "AI", "jobs", "automation", "skills"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-12T10:00:00Z",
    "views": 4567,
    "author": "Amanda Foster",
    "imageUrl": "https://images.unsplash.com/photo-1497032628192-86f99bcd76bc?w=800"
  }'::JSONB
);

-- ============================================================================
-- Theme 3: Web Development
-- ============================================================================

-- Rust Frameworks Compared (Similar to existing Rust article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/rust-frameworks-compared',
  'news:Article',
  'Rust Web Frameworks 2025: Actix vs Axum vs Rocket',
  '{
    "title": "Rust Web Frameworks 2025: Actix vs Axum vs Rocket",
    "slug": "rust-frameworks-compared",
    "excerpt": "A comprehensive comparison of the top Rust web frameworks for your next project.",
    "body": "# Rust Web Frameworks Compared\n\nChoosing the right Rust web framework? Heres our detailed comparison.\n\n## The Contenders\n\n### Actix Web\n- **Performance**: Fastest in benchmarks\n- **Maturity**: Most battle-tested\n- **Learning curve**: Steeper\n\n### Axum\n- **Performance**: Excellent (Tower-based)\n- **DX**: Best developer experience\n- **Ecosystem**: Growing rapidly\n\n### Rocket\n- **Performance**: Good\n- **DX**: Very intuitive\n- **Type safety**: Exceptional\n\n## Benchmark Results\n\n| Framework | Req/sec | Latency (p99) |\n|-----------|---------|---------------|\n| Actix | 450K | 1.2ms |\n| Axum | 420K | 1.4ms |\n| Rocket | 380K | 1.8ms |\n\n## Recommendation\n\n- **Microservices**: Actix or Axum\n- **Full-stack apps**: Rocket\n- **New projects**: Axum (best momentum)\n\n## Code Examples\n\nSee companion repo for implementation samples.",
    "keywords": ["Rust", "web frameworks", "Actix", "Axum", "Rocket"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/rust", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/advanced", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-20T09:00:00Z",
    "views": 3234,
    "author": "Elena Rodriguez",
    "imageUrl": "https://images.unsplash.com/photo-1555066931-4365d14bab8c?w=800"
  }'::JSONB
);

-- TypeScript vs JavaScript (Contradicts TypeScript 5 article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/typescript-vs-javascript',
  'news:Article',
  'TypeScript is Overrated: A Defense of Plain JavaScript',
  '{
    "title": "TypeScript is Overrated: A Defense of Plain JavaScript",
    "slug": "typescript-vs-javascript",
    "excerpt": "Hot take: TypeScript complexity often outweighs its benefits. Heres why.",
    "body": "# In Defense of JavaScript\n\nUnpopular opinion: TypeScript isnt always the answer.\n\n## The TypeScript Tax\n\n### Build Complexity\n- Extra compilation step\n- Config file management\n- Type definition maintenance\n\n### Learning Overhead\n- Generics complexity\n- Utility types confusion\n- Declaration files\n\n## When JavaScript Wins\n\n### Small Projects\n- Prototypes and MVPs\n- Scripts and utilities\n- Personal projects\n\n### Dynamic Use Cases\n- Heavy metaprogramming\n- Plugin systems\n- Runtime code generation\n\n## The Middle Ground\n\n### JSDoc Types\nGet type checking without TypeScript:\n```javascript\n/** @type {string} */\nlet name = \"Hello\";\n```\n\n## Conclusion\n\nUse TypeScript when:\n- Large teams\n- Long-lived codebases\n- Complex domains\n\nUse JavaScript when:\n- Speed matters\n- Flexibility needed\n- Learning is priority",
    "keywords": ["TypeScript", "JavaScript", "programming", "opinion"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/typescript", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-08T14:00:00Z",
    "views": 5678,
    "author": "Jake Thompson",
    "imageUrl": "https://images.unsplash.com/photo-1516116216624-53e697fedbea?w=800"
  }'::JSONB
);

-- Full Stack 2025 Part 1 (Story chain)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/full-stack-2025-part-1',
  'news:Article',
  'Full Stack Development 2025 Part 1: The Modern Stack',
  '{
    "title": "Full Stack Development 2025 Part 1: The Modern Stack",
    "slug": "full-stack-2025-part-1",
    "excerpt": "Part 1: Your complete guide to the modern full-stack development ecosystem.",
    "body": "# Full Stack Development 2025\n\nThe full-stack landscape has evolved dramatically. Heres your guide.\n\n## The Modern Stack\n\n### Frontend\n- **React/Next.js** - Still dominant\n- **Vue/Nuxt** - Strong alternative\n- **Svelte/SvelteKit** - Rising star\n\n### Backend\n- **Node.js** - JavaScript everywhere\n- **Go** - Performance focus\n- **Rust** - Systems web dev\n\n### Database\n- **PostgreSQL** - Default choice\n- **PlanetScale** - MySQL serverless\n- **Supabase** - Firebase alternative\n\n## Deployment\n\n### Edge Computing\n- Vercel Edge Functions\n- Cloudflare Workers\n- Deno Deploy\n\n### Traditional\n- AWS/GCP/Azure\n- Railway, Render\n- Self-hosted VPS\n\n*Part 2 covers practical implementation*",
    "keywords": ["full stack", "web development", "2025", "modern stack"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/typescript", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-18T10:00:00Z",
    "views": 2890,
    "author": "Elena Rodriguez",
    "imageUrl": "https://images.unsplash.com/photo-1461749280684-dccba630e2f6?w=800"
  }'::JSONB
);

-- Full Stack 2025 Part 2 (Continues Part 1)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/full-stack-2025-part-2',
  'news:Article',
  'Full Stack Development 2025 Part 2: Building a Complete App',
  '{
    "title": "Full Stack Development 2025 Part 2: Building a Complete App",
    "slug": "full-stack-2025-part-2",
    "excerpt": "Part 2: Hands-on guide to building a production-ready full-stack application.",
    "body": "# Building a Complete App\n\nLets put our modern stack knowledge into practice.\n\n## Project Setup\n\n### Tech Choices\n- **Frontend**: SvelteKit\n- **Backend**: Node.js + tRPC\n- **Database**: PostgreSQL + Drizzle\n- **Auth**: Lucia\n\n### Architecture\n```\nsrc/\n  routes/      # SvelteKit pages\n  lib/\n    server/    # Backend logic\n    client/    # Frontend utilities\n  db/          # Schema + migrations\n```\n\n## Key Features\n\n### Type Safety\nEnd-to-end types with tRPC:\n- No API schema duplication\n- Autocomplete everywhere\n- Refactoring confidence\n\n### Authentication\nLucia provides:\n- Session management\n- OAuth integrations\n- Security best practices\n\n## Deployment\n\nWell deploy to Vercel:\n1. Connect GitHub repo\n2. Configure environment\n3. Enable preview deployments\n\n## Source Code\n\nComplete code available on GitHub.",
    "keywords": ["full stack", "tutorial", "SvelteKit", "web development"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/typescript", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/advanced", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-25T10:00:00Z",
    "views": 1567,
    "author": "Elena Rodriguez",
    "imageUrl": "https://images.unsplash.com/photo-1517694712202-14dd9538aa97?w=800"
  }'::JSONB
);

-- WebAssembly Future (Similar to Rust article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/webassembly-future',
  'news:Article',
  'WebAssembly in 2025: Beyond the Browser',
  '{
    "title": "WebAssembly in 2025: Beyond the Browser",
    "slug": "webassembly-future",
    "excerpt": "WASM is escaping the browser. Heres how its reshaping computing.",
    "body": "# WebAssembly Beyond the Browser\n\nWebAssembly started as browser tech. Now its everywhere.\n\n## Server-Side WASM\n\n### WASI (WebAssembly System Interface)\n- File system access\n- Network capabilities\n- Cross-platform binaries\n\n### Use Cases\n- Edge functions (Cloudflare Workers)\n- Plugin systems (Envoy, VS Code)\n- Sandboxed execution\n\n## WASM Languages\n\n| Language | WASM Support | Size |\n|----------|-------------|------|\n| Rust | Excellent | Small |\n| Go | Good | Large |\n| C/C++ | Excellent | Small |\n| AssemblyScript | Native | Small |\n\n## Performance Reality\n\n- 2x faster than JavaScript (compute)\n- Near-native speed\n- Great for:\n  - Image processing\n  - Cryptography\n  - Games\n\n## Getting Started\n\nRecommended path:\n1. Learn Rust basics\n2. Try wasm-pack\n3. Build browser extension\n4. Explore WASI",
    "keywords": ["WebAssembly", "WASM", "Rust", "performance", "edge computing"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/rust", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/web", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-22T09:00:00Z",
    "views": 2345,
    "author": "Sarah Chen",
    "imageUrl": "https://images.unsplash.com/photo-1558494949-ef010cbdcc31?w=800"
  }'::JSONB
);

-- Tech Startup Ecosystem (Cross-category: tech + business)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/tech/tech-startup-ecosystem',
  'news:Article',
  'The 2025 Tech Startup Ecosystem: Trends and Opportunities',
  '{
    "title": "The 2025 Tech Startup Ecosystem: Trends and Opportunities",
    "slug": "tech-startup-ecosystem",
    "excerpt": "Where the opportunities are in tech startups this year.",
    "body": "# Tech Startup Ecosystem 2025\n\nThe startup landscape is evolving. Heres where to look.\n\n## Hot Sectors\n\n### AI Infrastructure\n- Model training platforms\n- Inference optimization\n- Data labeling tools\n\n### Climate Tech\n- Carbon capture\n- Grid optimization\n- Sustainable materials\n\n### Enterprise AI\n- Workflow automation\n- Document processing\n- Customer service AI\n\n## Funding Landscape\n\n### Seed Stage\n- Average: $3.5M\n- Typical dilution: 15-20%\n\n### Series A\n- Average: $15M\n- Typical dilution: 20-25%\n\n## Advice for Founders\n\n1. **Solve real problems** - Not solutions looking for problems\n2. **Distribution matters** - Tech alone isnt enough\n3. **Team diversity** - Different perspectives win\n4. **Capital efficiency** - Prove before scaling",
    "keywords": ["startups", "tech", "ecosystem", "trends", "opportunities"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/enterprise", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-14T10:00:00Z",
    "views": 3890,
    "author": "David Kim",
    "imageUrl": "https://images.unsplash.com/photo-1556761175-5973dc0f32e7?w=800"
  }'::JSONB
);

-- ============================================================================
-- Theme 4: Sports & Entertainment
-- ============================================================================

-- Olympics Venue Data (Provides Evidence for Olympic Preview)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/olympics-venue-data',
  'news:Article',
  'LA 2028 Venues: Complete Data and Capacity Analysis',
  '{
    "title": "LA 2028 Venues: Complete Data and Capacity Analysis",
    "slug": "olympics-venue-data",
    "excerpt": "Detailed breakdown of every venue for the 2028 Los Angeles Olympics.",
    "body": "# LA 2028 Venue Data\n\nComplete venue analysis for the upcoming Olympics.\n\n## Major Venues\n\n| Venue | Sport | Capacity | Status |\n|-------|-------|----------|--------|\n| SoFi Stadium | Ceremonies | 70,240 | Ready |\n| LA Coliseum | Track & Field | 77,500 | Renovation |\n| Crypto.com Arena | Basketball | 20,000 | Ready |\n| Rose Bowl | Soccer | 88,432 | Ready |\n\n## Budget Breakdown\n\n### Venue Costs\n- Existing venues: $1.2B renovation\n- New construction: $800M\n- Temporary facilities: $400M\n\n### Transportation\n- Metro expansion: $3B\n- Bus rapid transit: $500M\n- Bike infrastructure: $200M\n\n## Sustainability Metrics\n\n- 100% renewable energy target\n- Zero single-use plastics\n- Carbon neutral commitment\n\n## Data Sources\n\nLA28 Organizing Committee, IOC reports, city planning documents.",
    "keywords": ["Olympics", "LA 2028", "venues", "data", "sports"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-08T12:00:00Z",
    "views": 1893,
    "author": "Michael Torres",
    "imageUrl": "https://images.unsplash.com/photo-1569517282132-25d22f4573e6?w=800"
  }'::JSONB
);

-- Streaming Revenue Data (Provides Evidence for Streaming Wars)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/entertainment/streaming-revenue-data',
  'news:Article',
  'Streaming Platform Revenue 2025: The Complete Breakdown',
  '{
    "title": "Streaming Platform Revenue 2025: The Complete Breakdown",
    "slug": "streaming-revenue-data",
    "excerpt": "Detailed financial analysis of every major streaming platform.",
    "body": "# Streaming Revenue Analysis 2025\n\nComprehensive financial breakdown of the streaming industry.\n\n## Revenue by Platform\n\n| Platform | Revenue | YoY Growth | ARPU |\n|----------|---------|------------|------|\n| Netflix | $38B | +12% | $11.50 |\n| Disney+ | $22B | +18% | $8.20 |\n| Amazon Prime | $15B | +8% | $6.40 |\n| Max | $12B | +15% | $10.80 |\n| Peacock | $5B | +45% | $5.20 |\n\n## Key Metrics\n\n### Subscriber Costs\n- Content acquisition: 45% of revenue\n- Technology: 15%\n- Marketing: 20%\n- Operations: 10%\n\n### Profitability\n- Netflix: Profitable (15% margin)\n- Disney+: Breaking even\n- Others: Still investing\n\n## Trends\n\n1. Ad tier growth exceeding expectations\n2. Password sharing crackdowns working\n3. Sports driving subscriptions\n\n## Methodology\n\nData from SEC filings, earnings calls, and industry reports.",
    "keywords": ["streaming", "revenue", "data", "Netflix", "Disney+"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-19T15:00:00Z",
    "views": 2567,
    "author": "Rachel Green",
    "imageUrl": "https://images.unsplash.com/photo-1611162617474-5b21e879e113?w=800"
  }'::JSONB
);

-- Esports Salary Report (Provides Evidence for Esports article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/esports-salary-report',
  'news:Article',
  'Esports Salary Report 2025: How Much Do Pro Gamers Earn?',
  '{
    "title": "Esports Salary Report 2025: How Much Do Pro Gamers Earn?",
    "slug": "esports-salary-report",
    "excerpt": "Comprehensive salary data for professional esports players across all major titles.",
    "body": "# Esports Salary Report 2025\n\nWhat do professional gamers actually earn? Heres the data.\n\n## Average Salaries by Game\n\n| Game | Top Teams | Mid-Tier | Entry |\n|------|-----------|----------|-------|\n| League of Legends | $400K | $150K | $60K |\n| Valorant | $350K | $120K | $50K |\n| CS2 | $300K | $100K | $40K |\n| Dota 2 | $250K | $80K | $30K |\n\n## Total Compensation\n\n### Components\n- Base salary: 40%\n- Prize money: 30%\n- Sponsorships: 20%\n- Streaming: 10%\n\n### Top Earners 2025\n1. Faker (LoL): $5M+\n2. s1mple (CS2): $3M+\n3. TenZ (Valorant): $2.5M+\n\n## Career Length\n\n- Average pro career: 5-7 years\n- Peak performance: Ages 18-24\n- Post-career: Coaching, streaming, content\n\n## Data Sources\n\nTeam disclosures, agent interviews, tournament records.",
    "keywords": ["esports", "salary", "gaming", "professional", "data"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/analysis", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/audience/beginners", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-05T16:00:00Z",
    "views": 4789,
    "author": "Jason Park",
    "imageUrl": "https://images.unsplash.com/photo-1542751371-adc38448a05e?w=800"
  }'::JSONB
);

-- Traditional vs Esports (Contradicts Esports Mainstream)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/traditional-vs-esports',
  'news:Article',
  'Why Traditional Sports Will Always Beat Esports',
  '{
    "title": "Why Traditional Sports Will Always Beat Esports",
    "slug": "traditional-vs-esports",
    "excerpt": "A sports purists argument for why esports will never match traditional athletics.",
    "body": "# The Case for Traditional Sports\n\nEsports has grown, but it will never replace traditional athletics. Heres why.\n\n## The Physical Element\n\n### Athletic Excellence\n- Decades of training\n- Peak human performance\n- Physical courage and endurance\n\n### The Missing Piece\nWatching someone click a mouse lacks visceral excitement of:\n- A slam dunk\n- A goal-line save\n- A knockout punch\n\n## Cultural Legacy\n\n### Centuries of History\n- Olympics: 2,800 years\n- Football: 150+ years\n- Basketball: 130+ years\n- Esports: ~25 years\n\n### Community Roots\n- Local teams\n- School programs\n- Family traditions\n\n## Sustainability Questions\n\n### Game Lifespan\n- Sports rules: Stable for generations\n- Games: Can become obsolete\n\n### Publisher Control\n- Sports: No single owner\n- Esports: At mercy of game companies\n\n## Conclusion\n\nEsports is entertainment, but comparing it to traditional sports diminishes both.",
    "keywords": ["sports", "esports", "traditional", "comparison", "opinion"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/opinion", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-12T14:00:00Z",
    "views": 3456,
    "author": "Bob Johnson",
    "imageUrl": "https://images.unsplash.com/photo-1461896836934- voices-08f5d8b?w=800"
  }'::JSONB
);

-- Gaming Industry Correction (Corrects Esports article)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/sports/gaming-industry-correction',
  'news:Article',
  'Correction: Esports Industry Revenue Figures Updated',
  '{
    "title": "Correction: Esports Industry Revenue Figures Updated",
    "slug": "gaming-industry-correction",
    "excerpt": "Updated figures for the esports industry after methodology review.",
    "body": "# Correction Notice\n\nOur previous article on esports cited industry revenue of $1.8B. Updated analysis provides more accurate figures.\n\n## Corrected Data\n\n### Previous vs Updated\n| Metric | Original | Corrected |\n|--------|----------|----------|\n| Global Revenue | $1.8B | $1.4B |\n| Audience | 500M+ | 450M |\n| Pro Players | 100K+ | 75K |\n\n### Why the Difference?\n\n1. **Double counting** - Some revenue streams counted twice\n2. **Projection vs actual** - Used forecasts instead of actuals\n3. **Definition scope** - Included adjacent gaming revenue\n\n## Impact Assessment\n\nWhile numbers are lower, trends remain positive:\n- 15% YoY growth (not 22%)\n- Strong sponsorship growth\n- Improving unit economics\n\n## Our Standards\n\nWe strive for accuracy. When we make errors, we correct them promptly and transparently.",
    "keywords": ["esports", "correction", "gaming", "data", "industry"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/topics/breaking", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-18T09:00:00Z",
    "views": 1234,
    "author": "Jason Park",
    "imageUrl": "https://images.unsplash.com/photo-1542751371-adc38448a05e?w=800"
  }'::JSONB
);

-- AI in Entertainment (Cross-category)
UPSERT INTO social (path, node_type, name, properties) VALUES (
  '/superbigshit/articles/entertainment/ai-in-entertainment',
  'news:Article',
  'AI in Entertainment: From Script to Screen',
  '{
    "title": "AI in Entertainment: From Script to Screen",
    "slug": "ai-in-entertainment",
    "excerpt": "How artificial intelligence is transforming every aspect of entertainment production.",
    "body": "# AI in Entertainment\n\nFrom writing to distribution, AI is reshaping how entertainment is made.\n\n## Pre-Production\n\n### Script Analysis\n- Predicting box office potential\n- Identifying pacing issues\n- Character development suggestions\n\n### Casting\n- Digital de-aging\n- Voice cloning (with consent)\n- Performance capture enhancement\n\n## Production\n\n### Visual Effects\n- Real-time rendering\n- AI-assisted compositing\n- Deepfake technology (ethical use)\n\n### Sound Design\n- Automated foley\n- Music generation\n- Voice enhancement\n\n## Post-Production\n\n### Editing\n- Automated rough cuts\n- Continuity checking\n- Color grading assistance\n\n## Distribution\n\n### Personalization\n- Trailer variants by demographic\n- Thumbnail optimization\n- Release timing prediction\n\n## Industry Response\n\nSAG-AFTRA and WGA negotiations now include AI provisions.",
    "keywords": ["AI", "entertainment", "movies", "production", "technology"],
    "tags": [
      {"raisin:ref": "/superbigshit/tags/tech-stack/ai", "raisin:workspace": "social"},
      {"raisin:ref": "/superbigshit/tags/topics/trending", "raisin:workspace": "social"}
    ],
    "featured": false,
    "status": "published",
    "publishing_date": "2025-11-16T13:00:00Z",
    "views": 2345,
    "author": "Chris Williams",
    "imageUrl": "https://images.unsplash.com/photo-1485846234645-a62644f84728?w=800"
  }'::JSONB
);

-- ============================================================================
-- RELATE STATEMENTS: Create the Graph Network
-- These establish semantic connections between articles for graph traversal
-- ============================================================================

-- ============================================================================
-- STORY CHAINS (continues/updates)
-- These create linear narrative sequences that can be traversed as timelines
-- ============================================================================

-- AI Revolution 4-Part Series
RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'continues';

RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-3' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-2' IN WORKSPACE 'social'
  TYPE 'continues';

RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-4' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-3' IN WORKSPACE 'social'
  TYPE 'continues';

-- Full Stack Development 2-Part Series
RELATE FROM path='/superbigshit/articles/tech/full-stack-2025-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/full-stack-2025-part-1' IN WORKSPACE 'social'
  TYPE 'continues';

-- Startup Funding Series (Part 2 continues existing guide)
RELATE FROM path='/superbigshit/articles/business/startup-funding-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'continues';

-- ============================================================================
-- CORRECTIONS
-- These mark articles that fix/update information in previous articles
-- ============================================================================

-- AI Productivity Correction corrects AI Coding Assistants
RELATE FROM path='/superbigshit/articles/tech/ai-productivity-correction' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'corrects';

-- Gaming Industry Correction corrects Esports Mainstream
RELATE FROM path='/superbigshit/articles/sports/gaming-industry-correction' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/esports-mainstream' IN WORKSPACE 'social'
  TYPE 'corrects';

-- ============================================================================
-- CONTRADICTIONS (opposing viewpoints)
-- These connect articles presenting conflicting perspectives
-- ============================================================================

-- AI Skeptic contradicts AI Revolution Part 1
RELATE FROM path='/superbigshit/articles/tech/ai-skeptics-view' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'contradicts';

-- AI Skeptic also contradicts AI Coding Assistants
RELATE FROM path='/superbigshit/articles/tech/ai-skeptics-view' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'contradicts';

-- Bootstrapping vs VC contradicts Funding Guide
RELATE FROM path='/superbigshit/articles/business/bootstrapping-vs-vc' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'contradicts';

-- TypeScript vs JavaScript contradicts TypeScript 5 Features
RELATE FROM path='/superbigshit/articles/tech/typescript-vs-javascript' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/typescript-5-features' IN WORKSPACE 'social'
  TYPE 'contradicts';

-- Traditional vs Esports contradicts Esports Mainstream
RELATE FROM path='/superbigshit/articles/sports/traditional-vs-esports' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/esports-mainstream' IN WORKSPACE 'social'
  TYPE 'contradicts';

-- ============================================================================
-- EVIDENCE/SOURCES (supporting data)
-- These connect data-rich articles to the articles they support
-- ============================================================================

-- AI Ethics Data provides evidence for AI Revolution Part 1
RELATE FROM path='/superbigshit/articles/tech/ai-ethics-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- AI Ethics Data also provides evidence for AI Revolution Part 3 (Human Factor)
RELATE FROM path='/superbigshit/articles/tech/ai-ethics-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-3' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- VC Investment Data provides evidence for Startup Funding Guide
RELATE FROM path='/superbigshit/articles/business/vc-investment-q3-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- Remote Work Study provides evidence for Remote Work Trends
RELATE FROM path='/superbigshit/articles/business/remote-work-study' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/remote-work-trends' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- Olympics Venue Data provides evidence for Olympic Preview
RELATE FROM path='/superbigshit/articles/sports/olympics-venue-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/olympic-preview-2028' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- Streaming Revenue Data provides evidence for Streaming Wars
RELATE FROM path='/superbigshit/articles/entertainment/streaming-revenue-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/entertainment/streaming-wars-2025' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- Esports Salary Report provides evidence for Esports Mainstream
RELATE FROM path='/superbigshit/articles/sports/esports-salary-report' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/esports-mainstream' IN WORKSPACE 'social'
  TYPE 'provides-evidence-for';

-- ============================================================================
-- WEIGHTED SIMILARITY (content-based relationships)
-- These connect related content with relevance scores (0.0-1.0)
-- ============================================================================

-- AI-related articles
RELATE FROM path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.92;

RELATE FROM path='/superbigshit/articles/tech/ai-in-healthcare' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.88;

RELATE FROM path='/superbigshit/articles/tech/ai-in-healthcare' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.75;

RELATE FROM path='/superbigshit/articles/entertainment/music-ai-revolution' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.73;

RELATE FROM path='/superbigshit/articles/entertainment/music-ai-revolution' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.78;

-- Rust/Web Development related
RELATE FROM path='/superbigshit/articles/tech/rust-frameworks-compared' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/rust-web-development-2025' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.95;

RELATE FROM path='/superbigshit/articles/tech/webassembly-future' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/rust-web-development-2025' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.78;

RELATE FROM path='/superbigshit/articles/tech/rust-web-development-2025' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/typescript-5-features' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.65;

-- TypeScript/JavaScript related
RELATE FROM path='/superbigshit/articles/tech/full-stack-2025-part-1' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/typescript-5-features' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.82;

RELATE FROM path='/superbigshit/articles/tech/full-stack-2025-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/typescript-5-features' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.80;

-- Business/Startup related
RELATE FROM path='/superbigshit/articles/business/startup-failures' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.85;

RELATE FROM path='/superbigshit/articles/business/vc-investment-q3-data' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.90;

RELATE FROM path='/superbigshit/articles/business/remote-work-study' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/remote-work-trends' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.92;

-- ============================================================================
-- EDITORIAL SEE-ALSO (curated recommendations)
-- These are editorial picks for related reading
-- ============================================================================

-- Startup ecosystem connections
RELATE FROM path='/superbigshit/articles/business/startup-failures' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.80;

RELATE FROM path='/superbigshit/articles/tech/tech-startup-ecosystem' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-funding-guide' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.85;

RELATE FROM path='/superbigshit/articles/tech/tech-startup-ecosystem' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/vc-investment-q3-data' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.82;

-- Web development see-also
RELATE FROM path='/superbigshit/articles/tech/full-stack-2025-part-1' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/rust-web-development-2025' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.75;

RELATE FROM path='/superbigshit/articles/tech/webassembly-future' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/rust-frameworks-compared' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.80;

-- AI series see-also
RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-in-healthcare' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.85;

RELATE FROM path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-2' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.78;

-- ============================================================================
-- CROSS-CATEGORY CONNECTIONS
-- These link articles across different topics/categories
-- ============================================================================

-- AI in Entertainment connects tech and entertainment
RELATE FROM path='/superbigshit/articles/entertainment/ai-in-entertainment' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-1' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.70;

RELATE FROM path='/superbigshit/articles/entertainment/ai-in-entertainment' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.65;

-- Future of Work connects business and tech
RELATE FROM path='/superbigshit/articles/business/future-work-ai' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.75;

RELATE FROM path='/superbigshit/articles/business/future-work-ai' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-revolution-part-3' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.88;

RELATE FROM path='/superbigshit/articles/business/future-work-ai' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/remote-work-trends' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.72;

-- Tech startup ecosystem bridges tech and business
RELATE FROM path='/superbigshit/articles/tech/tech-startup-ecosystem' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/bootstrapping-vs-vc' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.70;

-- Music AI connects entertainment and AI
RELATE FROM path='/superbigshit/articles/entertainment/music-ai-revolution' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/entertainment/streaming-wars-2025' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.68;

-- Esports and streaming connection
RELATE FROM path='/superbigshit/articles/sports/esports-mainstream' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/entertainment/streaming-wars-2025' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.65;

-- ============================================================================
-- ADDITIONAL WEIGHTED CONNECTIONS FOR RICH GRAPH
-- More connections to make the demo network impressive
-- ============================================================================

-- Connect AI articles with varying weights
RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-coding-assistants' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.82;

RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-3' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/future-work-ai' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.90;

RELATE FROM path='/superbigshit/articles/tech/ai-revolution-part-4' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/ai-skeptics-view' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.75;

-- Connect business articles
RELATE FROM path='/superbigshit/articles/business/bootstrapping-vs-vc' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-failures' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.78;

RELATE FROM path='/superbigshit/articles/business/startup-funding-part-2' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/business/startup-failures' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.72;

-- Connect web development articles
RELATE FROM path='/superbigshit/articles/tech/typescript-5-features' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/full-stack-2025-part-1' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.78;

RELATE FROM path='/superbigshit/articles/tech/rust-web-development-2025' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/tech/webassembly-future' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.82;

-- Connect sports articles
RELATE FROM path='/superbigshit/articles/sports/esports-mainstream' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/esports-salary-report' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.88;

RELATE FROM path='/superbigshit/articles/sports/olympic-preview-2028' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/sports/traditional-vs-esports' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.55;

-- Connect entertainment articles
RELATE FROM path='/superbigshit/articles/entertainment/streaming-wars-2025' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/entertainment/streaming-revenue-data' IN WORKSPACE 'social'
  TYPE 'see-also' WEIGHT 0.90;

RELATE FROM path='/superbigshit/articles/entertainment/music-ai-revolution' IN WORKSPACE 'social'
  TO path='/superbigshit/articles/entertainment/ai-in-entertainment' IN WORKSPACE 'social'
  TYPE 'similar-to' WEIGHT 0.85;

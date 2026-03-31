-- INSERT statements for creating nodes

-- Basic INSERT with path, node_type, and properties
INSERT INTO nodes (path, node_type, properties)
VALUES ('/content/blog/my-first-post', 'my:Article', '{"title": "Hello World", "status": "draft"}');

-- INSERT with multiple columns
INSERT INTO nodes (path, node_type, properties, created_by)
VALUES ('/content/pages/about', 'my:Page', '{"title": "About Us"}', 'user-123');

-- INSERT with complex JSON properties
INSERT INTO nodes (path, node_type, properties)
VALUES (
    '/products/laptop-pro',
    'my:Product',
    '{"title": "Laptop Pro", "price": 1299.99, "specs": {"ram": "16GB", "cpu": "Intel i7"}}'
);

-- INSERT with owner_id
INSERT INTO nodes (path, node_type, properties, owner_id)
VALUES ('/workspace/project1', 'my:Project', '{"name": "Project Alpha"}', 'org-456');

-- INSERT with translations
INSERT INTO nodes (path, node_type, properties, translations)
VALUES (
    '/content/blog/multilingual-post',
    'my:Article',
    '{"title": "Welcome"}',
    '{"de": {"title": "Willkommen"}, "fr": {"title": "Bienvenue"}}'
);

-- INSERT array of tags in properties
INSERT INTO nodes (path, node_type, properties)
VALUES (
    '/content/blog/tech-article',
    'my:Article',
    '{"title": "Tech Trends", "tags": ["technology", "innovation", "AI"], "status": "published"}'
);

-- INSERT with nested path
INSERT INTO nodes (path, node_type, properties)
VALUES (
    '/projects/2025/q1/milestone-1',
    'my:Milestone',
    '{"name": "Launch MVP", "deadline": "2025-03-31"}'
);

-- INSERT with SEO metadata
INSERT INTO nodes (path, node_type, properties)
VALUES (
    '/content/landing-page',
    'my:Page',
    '{"title": "Home", "seo": {"title": "Welcome to Our Site", "description": "Best products online"}}'
);

-- INSERT minimal required fields
INSERT INTO nodes (path, node_type)
VALUES ('/content/simple-page', 'my:Page');

-- INSERT with author metadata
INSERT INTO nodes (path, node_type, properties, created_by, owner_id)
VALUES (
    '/blog/guest-post',
    'my:Article',
    '{"title": "Guest Article", "author": "Jane Doe"}',
    'user-789',
    'user-789'
);

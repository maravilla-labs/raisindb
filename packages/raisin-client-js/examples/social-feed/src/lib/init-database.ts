/**
 * Database initialization script
 * Run this with: npm run init-db
 */

import { getClient, getConfig } from './raisin';

async function initializeDatabase() {
  console.log('🚀 Initializing RaisinDB Social Feed database...\n');

  const client = await getClient();
  const config = getConfig();

  try {
    // Step 1: Delete and recreate repository (to ensure clean slate)
    console.log('📦 Setting up repository...');
    try {
      console.log('  🗑️  Deleting existing repository (if any)...');
      await client.deleteRepository(config.repository);
      console.log('  ✅ Old repository deleted');
    } catch (error: any) {
      if (error.message?.includes('not found')) {
        console.log('  ℹ️  No existing repository to delete');
      } else {
        console.log('  ⚠️  Could not delete repository:', error.message);
      }
    }

    try {
      await client.createRepository(config.repository, 'Social Feed Demo Repository');
      console.log('  ✅ Repository created fresh\n');
    } catch (error: any) {
      throw error;
    }

    const db = client.database(config.repository);

    // Step 2: Create workspace
    console.log('🗂️  Creating workspace...');
    try {
      await db.createWorkspace(config.workspace, 'Social Feed Workspace');
      console.log('✅ Workspace created and ready\n');
    } catch (error: any) {
      throw error;
    }

    // Step 3: Define NodeTypes
    console.log('📋 Creating NodeTypes...');

    const userNodeType = {
      name: 'SocialUser',
      description: 'A user in the social network',
      properties: [
        {
          name: 'username',
          type: 'String',
          required: true,
          unique: true,
        },
        {
          name: 'displayName',
          type: 'String',
          required: true,
        },
        {
          name: 'bio',
          type: 'String',
        },
        {
          name: 'avatar',
          type: 'String',
        },
        {
          name: 'followerCount',
          type: 'Number',
          default: 0,
        },
        {
          name: 'followingCount',
          type: 'Number',
          default: 0,
        },
      ],
      allowed_children: [],
    };

    const postNodeType = {
      name: 'Post',
      description: 'A social media post',
      properties: [
        {
          name: 'content',
          type: 'String',
          required: true,
          constraints: { maxLength: 280 },
        },
        {
          name: 'likeCount',
          type: 'Number',
          default: 0,
        },
        {
          name: 'commentCount',
          type: 'Number',
          default: 0,
        },
      ],
      allowed_children: ['Comment'],
    };

    const commentNodeType = {
      name: 'Comment',
      description: 'A comment on a post',
      properties: [
        {
          name: 'content',
          type: 'String',
          required: true,
          constraints: { maxLength: 280 },
        },
        {
          name: 'likeCount',
          type: 'Number',
          default: 0,
        },
      ],
      allowed_children: [],
    };

    // Create NodeTypes
    for (const nodeType of [userNodeType, postNodeType, commentNodeType]) {
      try {
        await db.nodeTypes().create(nodeType.name, nodeType);
        await db.nodeTypes().publish(nodeType.name);
        console.log(`  ✅ ${nodeType.name} NodeType created and published`);
      } catch (error: any) {
        if (error.message?.includes('already exists')) {
          console.log(`  ℹ️  ${nodeType.name} NodeType already exists`);
        } else {
          throw error;
        }
      }
    }
    console.log();

    // Step 4: Create nodes in a transaction
    console.log('📦 Creating nodes in a transaction...\n');
    const ws = db.workspace(config.workspace);
    const tx = ws.transaction();

    // Begin transaction
    await tx.begin({ message: 'Initialize social feed demo data' });

    // Step 5: Create demo users
    console.log('👥 Creating demo users...');

    const demoUsers = [
      {
        username: 'alice',
        displayName: 'Alice Johnson',
        bio: 'Software engineer and tech enthusiast 👩‍💻',
        avatar: '👩',
      },
      {
        username: 'bob',
        displayName: 'Bob Smith',
        bio: 'Design thinking and UX advocate 🎨',
        avatar: '👨',
      },
      {
        username: 'carol',
        displayName: 'Carol Williams',
        bio: 'Data scientist exploring the world of AI 🤖',
        avatar: '👩‍🔬',
      },
    ];
    await tx.nodes().create({
          type: 'raisin:Folder',
          path: `/users`,
        });

    const userNodes = [];
    try {
      for (const user of demoUsers) {
        const node = await tx.nodes().create({
          type: 'SocialUser',
          path: `/users/${user.username}`,
          properties: user,
        });
        userNodes.push(node);
        console.log(`  ✅ Created user: ${user.displayName}`);
      }
    } catch (error) {
      console.error('❌ Error creating users, rolling back transaction:', error);
      await tx.rollback();
      throw error;
    }
    console.log();

    // Step 6: Create demo posts in transaction
    console.log('📝 Creating demo posts...');
    const demoPosts = [
      {
        author: userNodes[0].id,
        authorName: 'alice',
        content: 'Just deployed my first app using RaisinDB! The real-time features are amazing 🚀',
      },
      {
        author: userNodes[1].id,
        authorName: 'bob',
        content: 'Working on a new design system. Love how hierarchical data models simplify everything!',
      },
      {
        author: userNodes[2].id,
        authorName: 'carol',
        content: 'Training a new ML model. The graph queries in RaisinDB make relationship analysis so easy!',
      },
    ];

    // Create posts folder for each user to leverage hierarchical paths
    for (const user of demoUsers) {
        await tx.nodes().create({
            type: 'raisin:Folder',
            path: `/users/${user.username}/posts`,
        });
    }

    const createdPosts = [];

    try {
      for (const post of demoPosts) {
        // Use hierarchical path: /users/{username}/posts/{post_id}
        // This enables efficient "get all posts by user" via prefix scan or path traversal
        const postPath = `/users/${post.authorName}/posts/${Date.now()}_${Math.floor(Math.random()*1000)}`;
        
        const postNode = await tx.nodes().create({
          type: 'Post',
          path: postPath,
          properties: {
            content: post.content,
            authorId: post.author,
            likeCount: Math.floor(Math.random() * 10),
            commentCount: 0,
          },
        });
        createdPosts.push({ node: postNode, authorPath: `/users/${post.authorName}` });
        console.log(`  ✅ Created post by ${post.authorName} at ${postPath}`);
      }
    } catch (error) {
      console.error('❌ Error creating posts, rolling back transaction:', error);
      await tx.rollback();
      throw error;
    }
    console.log();

    // Step 7: Commit the transaction
    console.log('💾 Committing transaction...');
    try {
      const result = await tx.commit();
      console.log(`✅ Transaction committed - Revision: ${result.revision}, Commit ID: ${result.commit_id}\n`);
    } catch (error) {
      console.error('❌ Error committing transaction:', error);
      throw error;
    }

    // Step 8: Create relationships
    console.log('🔗 Creating relationships...');
    
    // Create AUTHORED relationships
    for (const post of createdPosts) {
        try {
            await ws.nodes().addRelation(
                post.authorPath,
                'AUTHORED',
                post.node.path
            );
            console.log(`  ✅ Created AUTHORED relation for ${post.authorPath}`);
        } catch (error) {
            console.log(`  ⚠️ Failed to create AUTHORED relation: ${error}`);
        }
    }

    if (userNodes.length >= 3) {
      // Alice follows Bob and Carol
      // Bob follows Alice
      // Carol follows Alice and Bob
      const relationships = [
        { fromPath: userNodes[0].path, toPath: userNodes[1].path, follower: 'alice', following: 'bob' },
        { fromPath: userNodes[0].path, toPath: userNodes[2].path, follower: 'alice', following: 'carol' },
        { fromPath: userNodes[1].path, toPath: userNodes[0].path, follower: 'bob', following: 'alice' },
        { fromPath: userNodes[2].path, toPath: userNodes[0].path, follower: 'carol', following: 'alice' },
        { fromPath: userNodes[2].path, toPath: userNodes[1].path, follower: 'carol', following: 'bob' },
      ];

      for (const rel of relationships) {
        try {
          await ws.nodes().addRelation(
            rel.fromPath,
            'FOLLOWS', // Uppercase convention for relations
            rel.toPath
          );
          console.log(`  ✅ ${rel.follower} FOLLOWS ${rel.following}`);
        } catch (error) {
          console.log(`  ℹ️  Relationship ${rel.follower} -> ${rel.following} may already exist`);
        }
      }
    }
    console.log();

    console.log('✨ Database initialization complete!');
    console.log('\n🎉 You can now run: npm run dev\n');

  } catch (error) {
    console.error('❌ Error initializing database:', error);
    process.exit(1);
  } finally {
    client.disconnect();
  }
}

// Run if executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  initializeDatabase();
}

export { initializeDatabase };

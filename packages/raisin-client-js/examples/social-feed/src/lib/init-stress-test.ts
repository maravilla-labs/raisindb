/**
 * Stress test initialization script
 * Creates 1M posts to test database performance at scale
 * Run this with: npm run init-stresstest
 */

import { getClient, getConfig } from './raisin';

// Configuration
const TOTAL_POSTS = 1_000_000;
const BATCH_SIZE = 10_000; // Posts per transaction (optimized for RocksDB)
const BATCH_DELAY_MS = 300; // Delay between batches
const FOLDER_BATCH_SIZE = 500; // Folders per transaction
const TOTAL_BATCHES = Math.ceil(TOTAL_POSTS / BATCH_SIZE);

/**
 * Calculate bucket paths for a post number using range-ceiling naming
 * Post N goes into:
 * - Thousand bucket: Math.ceil(N / 1000) * 1000
 * - Hundred bucket: Math.ceil((position within thousand) / 100) * 100
 */
function calculateBuckets(postNumber: number): { thousand: number; hundred: number } {
  const thousandBucket = Math.ceil(postNumber / 1000) * 1000;
  const positionInThousand = ((postNumber - 1) % 1000) + 1; // 1-1000
  const hundredBucket = Math.ceil(positionInThousand / 100) * 100;
  return { thousand: thousandBucket, hundred: hundredBucket };
}

/**
 * Get the full hierarchical path for a post
 * Examples:
 * - Post 1 → /posts/1000/100/post-1
 * - Post 100 → /posts/1000/100/post-100
 * - Post 101 → /posts/1000/200/post-101
 * - Post 1001 → /posts/2000/100/post-1001
 */
function getPostPath(postNumber: number): string {
  const { thousand, hundred } = calculateBuckets(postNumber);
  return `/posts/${thousand}/${hundred}/post-${postNumber}`;
}

/**
 * Generate random post content
 */
function generateRandomContent(postNumber: number): string {
  const topics = [
    'Just deployed a new feature with RaisinDB! 🚀',
    'Working on scaling our database infrastructure today.',
    'Loving the graph query capabilities of RaisinDB!',
    'Performance benchmarks looking great with the new indexing.',
    'Real-time updates are working flawlessly across the cluster.',
    'Just hit a new milestone with our distributed system!',
    'The transaction support makes data consistency so easy.',
    'Exploring new patterns for hierarchical data modeling.',
    'WebSocket protocol is incredibly efficient for our use case.',
    'Successfully migrated 1M+ records to the new schema.',
  ];

  const emojis = ['🎉', '🚀', '💡', '⚡', '🔥', '✨', '💪', '🌟', '📊', '🎯'];

  const topic = topics[Math.floor(Math.random() * topics.length)];
  const emoji = emojis[Math.floor(Math.random() * emojis.length)];

  return `${topic} Post #${postNumber} ${emoji}`;
}

/**
 * Sleep for a specified number of milliseconds
 */
function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Create thousand-bucket folders in batches
 * Creates folders like /posts/1000, /posts/2000, ..., /posts/40000
 */
async function createThousandBucketFolders(workspace: any): Promise<void> {
  const totalThousandBuckets = Math.ceil(TOTAL_POSTS / 1000); // 40 buckets
  const batches = Math.ceil(totalThousandBuckets / FOLDER_BATCH_SIZE);

  console.log(`📁 Creating ${totalThousandBuckets} thousand-bucket folders...`);

  for (let batchNum = 0; batchNum < batches; batchNum++) {
    const tx = workspace.transaction();
    const startIdx = batchNum * FOLDER_BATCH_SIZE;
    const endIdx = Math.min(startIdx + FOLDER_BATCH_SIZE, totalThousandBuckets);

    try {
      await tx.begin({
        message: `Create thousand-bucket folders batch ${batchNum + 1}/${batches}`
      });

      for (let i = startIdx; i < endIdx; i++) {
        const thousandBucket = (i + 1) * 1000; // 1000, 2000, ..., 40000
        await tx.nodes().create({
          type: 'raisin:Folder',
          path: `/posts/${thousandBucket}`,
        });
      }

      await tx.commit();
      console.log(`  ✅ Created thousand buckets ${startIdx + 1}-${endIdx} ` +
                  `(folders: /posts/${(startIdx + 1) * 1000} to /posts/${endIdx * 1000})`);
    } catch (error: any) {
      if (tx.isTransactionActive()) {
        await tx.rollback();
      }
      // If folders already exist, that's acceptable
      if (error.message?.includes('already exists')) {
        console.log(`  ℹ️  Thousand bucket folders ${startIdx + 1}-${endIdx} already exist`);
      } else {
        throw error;
      }
    }
  }
}

/**
 * Create hundred-bucket folders in batches
 * Creates folders like /posts/1000/100, /posts/1000/200, ..., /posts/40000/1000
 */
async function createHundredBucketFolders(workspace: any): Promise<void> {
  const totalThousandBuckets = Math.ceil(TOTAL_POSTS / 1000); // 40
  const totalHundredBuckets = totalThousandBuckets * 10; // 400 (10 hundred buckets per thousand)
  const batches = Math.ceil(totalHundredBuckets / FOLDER_BATCH_SIZE);

  console.log(`📁 Creating ${totalHundredBuckets} hundred-bucket folders...`);

  for (let batchNum = 0; batchNum < batches; batchNum++) {
    const tx = workspace.transaction();
    const startIdx = batchNum * FOLDER_BATCH_SIZE;
    const endIdx = Math.min(startIdx + FOLDER_BATCH_SIZE, totalHundredBuckets);

    try {
      await tx.begin({
        message: `Create hundred-bucket folders batch ${batchNum + 1}/${batches}`
      });

      for (let i = startIdx; i < endIdx; i++) {
        const thousandIdx = Math.floor(i / 10); // Which thousand bucket (0-39)
        const hundredIdx = i % 10; // Which hundred within that thousand (0-9)
        const thousandBucket = (thousandIdx + 1) * 1000; // 1000, 2000, etc.
        const hundredBucket = (hundredIdx + 1) * 100; // 100, 200, ..., 1000

        await tx.nodes().create({
          type: 'raisin:Folder',
          path: `/posts/${thousandBucket}/${hundredBucket}`,
        });
      }

      await tx.commit();
      console.log(`  ✅ Created hundred buckets ${startIdx + 1}-${endIdx}`);
    } catch (error: any) {
      if (tx.isTransactionActive()) {
        await tx.rollback();
      }
      // If folders already exist, that's acceptable
      if (error.message?.includes('already exists')) {
        console.log(`  ℹ️  Hundred bucket folders ${startIdx + 1}-${endIdx} already exist`);
      } else {
        throw error;
      }
    }
  }
}

/**
 * Create a batch of posts in a single transaction
 * Returns true on success, false on failure (does not throw)
 */
async function createPostBatch(
  workspace: any,
  batchNumber: number,
  startIndex: number,
  count: number
): Promise<boolean> {
  const tx = workspace.transaction();

  try {
    await tx.begin({
      message: `Stress test batch ${batchNumber}/${TOTAL_BATCHES} (posts ${startIndex + 1}-${startIndex + count})`,
    });

    // Create all posts in this batch
    for (let i = 0; i < count; i++) {
      const postIndex = startIndex + i;
      const postNumber = postIndex + 1; // Posts are 1-indexed for path calculation
      const content = generateRandomContent(postNumber);
      const path = getPostPath(postNumber);

      await tx.nodes().create({
        type: 'Post',
        path: path,
        properties: {
          content: content,
          likeCount: Math.floor(Math.random() * 100),
          commentCount: Math.floor(Math.random() * 20),
        },
      });
    }

    // Commit the transaction
    const result = await tx.commit();
    console.log(
      `  ✅ Batch ${batchNumber}/${TOTAL_BATCHES} committed - ` +
      `Posts ${startIndex + 1}-${startIndex + count} created ` +
      `(Revision: ${result.revision})`
    );
    return true;
  } catch (error: any) {
    const errorMessage = error?.message || String(error);
    console.error(`  ❌ Error in batch ${batchNumber}:`, errorMessage);

    // Try to rollback if transaction is active
    try {
      if (tx.isTransactionActive()) {
        await tx.rollback();
        console.log(`  ↩️  Batch ${batchNumber} rolled back`);
      }
    } catch (rollbackError: any) {
      console.warn(`  ⚠️  Rollback failed for batch ${batchNumber}:`, rollbackError?.message);
    }

    return false;
  }
}

/**
 * Main stress test function
 */
async function runStressTest() {
  console.log('🏋️  Starting RaisinDB Stress Test\n');
  console.log(`📊 Configuration:`);
  console.log(`   • Total Posts: ${TOTAL_POSTS.toLocaleString()}`);
  console.log(`   • Batch Size: ${BATCH_SIZE.toLocaleString()} posts/transaction`);
  console.log(`   • Batch Delay: ${BATCH_DELAY_MS}ms between batches`);
  console.log(`   • Total Batches: ${TOTAL_BATCHES}`);
  console.log(`   • Hierarchical Structure: 3-level (thousand/hundred/post)`);
  console.log(`   • Max Nodes per Level: 1000 (balanced tree)\n`);

  const client = await getClient();
  const config = getConfig();

  try {
    const db = client.database(config.repository);
    const ws = db.workspace(config.workspace);

    // Verify /posts folder exists
    console.log('📁 Checking /posts folder...');
    try {
      const postsFolder = await ws.nodes().getByPath('/posts');
      if (!postsFolder) {
        console.log('   ℹ️  /posts folder not found, creating it...');
        const tx = ws.transaction();
        try {
          await tx.begin({ message: 'Create /posts folder for stress test' });
          await tx.nodes().create({
            type: 'raisin:Folder',
            path: '/posts',
          });
          await tx.commit();
          console.log('   ✅ /posts folder created');
        } catch (createError: any) {
          // If creation fails (e.g., already exists), rollback the transaction
          if (tx.isTransactionActive()) {
            await tx.rollback();
          }
          // If it already exists, that's fine
          if (createError.message?.includes('already exists')) {
            console.log('   ✅ /posts folder already exists');
          } else {
            throw createError;
          }
        }
      } else {
        console.log('   ✅ /posts folder exists');
      }
    } catch (error: any) {
      // If the query itself fails, /posts might not exist
      // Try to create it in a transaction
      console.log('   ℹ️  /posts folder not found, creating it...');
      const tx = ws.transaction();
      try {
        await tx.begin({ message: 'Create /posts folder for stress test' });
        await tx.nodes().create({
          type: 'raisin:Folder',
          path: '/posts',
        });
        await tx.commit();
        console.log('   ✅ /posts folder created');
      } catch (createError: any) {
        if (tx.isTransactionActive()) {
          await tx.rollback();
        }
        // If it already exists, that's fine
        if (createError.message?.includes('already exists')) {
          console.log('   ✅ /posts folder already exists');
        } else {
          console.log('   ⚠️  Could not create /posts folder:', createError.message);
        }
      }
    }
    console.log();

    // Create hierarchical folder structure
    console.log('🗂️  Creating hierarchical folder structure...\n');
    await createThousandBucketFolders(ws);
    console.log();
    await createHundredBucketFolders(ws);
    console.log();

    // Start timing
    const startTime = Date.now();
    console.log(`⏱️  Starting post creation at ${new Date().toLocaleTimeString()}\n`);

    // Track success/failure
    let successfulBatches = 0;
    let failedBatches = 0;

    // Create posts in batches
    for (let batchNum = 1; batchNum <= TOTAL_BATCHES; batchNum++) {
      const startIndex = (batchNum - 1) * BATCH_SIZE;
      const remainingPosts = TOTAL_POSTS - startIndex;
      const postsInBatch = Math.min(BATCH_SIZE, remainingPosts);

      // Progress indicator
      const progress = ((batchNum - 1) / TOTAL_BATCHES * 100).toFixed(1);
      console.log(`📦 Batch ${batchNum}/${TOTAL_BATCHES} (${progress}% complete)`);

      const success = await createPostBatch(ws, batchNum, startIndex, postsInBatch);
      if (success) {
        successfulBatches++;
      } else {
        failedBatches++;
      }

      // Wait between batches to avoid overwhelming the server
      if (batchNum < TOTAL_BATCHES) {
        console.log(`   ⏳ Waiting ${BATCH_DELAY_MS}ms before next batch...`);
        await sleep(BATCH_DELAY_MS);
      }

      // Show intermediate timing stats every 10 batches
      if (batchNum % 10 === 0) {
        const elapsed = Date.now() - startTime;
        const postsCreated = successfulBatches * BATCH_SIZE;
        const postsPerSecond = (postsCreated / (elapsed / 1000)).toFixed(2);
        const estimatedTotal = (elapsed / batchNum * TOTAL_BATCHES / 1000).toFixed(1);

        console.log(
          `   📈 Progress: ${postsCreated.toLocaleString()}/${TOTAL_POSTS.toLocaleString()} posts ` +
          `(${postsPerSecond} posts/sec, ETA: ${estimatedTotal}s remaining)\n`
        );
      }
    }

    // Final statistics
    const totalTime = Date.now() - startTime;
    const totalSeconds = (totalTime / 1000).toFixed(2);
    const postsCreated = successfulBatches * BATCH_SIZE;
    const postsPerSecond = (postsCreated / (totalTime / 1000)).toFixed(2);

    // Calculate folder statistics
    const totalThousandBuckets = Math.ceil(TOTAL_POSTS / 1000);
    const totalHundredBuckets = totalThousandBuckets * 10;
    const totalFolders = 1 + totalThousandBuckets + totalHundredBuckets; // root + thousand + hundred

    console.log('\n✨ Stress Test Complete!\n');
    console.log('📊 Final Statistics:');
    console.log(`   • Posts Created: ${postsCreated.toLocaleString()}/${TOTAL_POSTS.toLocaleString()}`);
    console.log(`   • Successful Batches: ${successfulBatches}/${TOTAL_BATCHES}`);
    console.log(`   • Failed Batches: ${failedBatches}`);
    console.log(`   • Total Folders Created: ${totalFolders.toLocaleString()} (1 root + ${totalThousandBuckets} thousand + ${totalHundredBuckets} hundred)`);
    console.log(`   • Total Time: ${totalSeconds}s`);
    console.log(`   • Average Rate: ${postsPerSecond} posts/second`);

    if (failedBatches > 0) {
      console.log(`\n⚠️  Completed with ${failedBatches} failed batches\n`);
    } else {
      console.log(`\n🎉 All ${TOTAL_POSTS.toLocaleString()} posts successfully created!\n`);
    }

  } catch (error) {
    console.error('\n❌ Unexpected error:', error);
  } finally {
    client.disconnect();
  }
}

// Run if executed directly
if (import.meta.url === `file://${process.argv[1]}`) {
  runStressTest();
}

export { runStressTest };

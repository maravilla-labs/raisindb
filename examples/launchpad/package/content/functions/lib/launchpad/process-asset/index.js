/**
 * Process Asset
 *
 * AI-powered asset processing function that demonstrates the Resource API.
 * Generates alt-text, descriptions, keywords, and thumbnails for uploaded images.
 *
 * This function is triggered when a raisin:Asset is updated and has a
 * storage_key (meaning the file upload is complete).
 */
async function processAsset(context) {
  console.log('[process-asset] === FUNCTION STARTED ===');
  console.log('[process-asset] context.flow_input:', JSON.stringify(context.flow_input, null, 2));

  const { event, workspace } = context.flow_input;

  console.log('[process-asset] Event type:', event.event_type);
  console.log('[process-asset] Workspace:', workspace);
  console.log('[process-asset] Node path:', event.node_path);
  console.log('[process-asset] Node ID:', event.node_id);

  // Get the full node with resource methods
  console.log('[process-asset] Fetching node from workspace...');
  const node = await raisin.nodes.get(workspace, event.node_path);

  if (!node) {
    console.error('[process-asset] Node not found:', event.node_path);
    return { success: false, error: 'Node not found' };
  }

  console.log('[process-asset] Node fetched successfully');
  console.log('[process-asset] Node ID:', node.id);
  console.log('[process-asset] Node type:', node.node_type);
  console.log('[process-asset] Node properties keys:', Object.keys(node.properties || {}));

  // Get the file resource
  console.log('[process-asset] Getting file resource...');
  const resource = node.getResource('./file');

  if (!resource) {
    console.log('[process-asset] No file resource found, skipping');
    console.log('[process-asset] Available properties:', JSON.stringify(node.properties, null, 2));
    return { skipped: true, reason: 'No file resource' };
  }

  console.log('[process-asset] Resource found:');
  console.log('[process-asset]   - name:', resource.name);
  console.log('[process-asset]   - mimeType:', resource.mimeType);
  console.log('[process-asset]   - size:', resource.size);
  console.log('[process-asset]   - storageKey:', resource.storageKey);

  // Check if it's an image or PDF
  const isImage = resource.mimeType?.startsWith('image/');
  const isPdf = resource.mimeType === 'application/pdf';

  if (!isImage && !isPdf) {
    console.log('[process-asset] Not an image or PDF, skipping:', resource.mimeType);
    return { skipped: true, reason: 'Not an image or PDF' };
  }

  // Process PDF files
  if (isPdf) {
    console.log('[process-asset] === PROCESSING PDF ===');
    return await processPdf(node, resource, workspace);
  }

  console.log('[process-asset] === PROCESSING IMAGE ===');

  const results = {
    success: true,
    description: null,
    alt_text: null,
    keywords: null,
    thumbnail_created: false,
    embedding_generated: false,
  };

  try {
    // Resize image for API limits (most vision models handle up to 2048px well)
    console.log('[process-asset] Resizing image for processing (maxWidth: 2048)...');
    const resized = await resource.resize({ maxWidth: 2048 });
    console.log('[process-asset] Resize complete, getting binary data...');
    const base64 = await resized.getBinary();
    console.log('[process-asset] Binary data retrieved, base64 length:', base64?.length || 0);

    // ========================================
    // STAGE 1: Get raw description from vision model (local:moondream)
    // ========================================
    console.log('[process-asset] === STAGE 1: VISION PROCESSING ===');
    console.log('[process-asset] Calling local:moondream for raw image description...');

    let rawDescription = null;
    try {
      const visionResponse = await raisin.ai.completion({
        model: 'local:moondream',
        messages: [{
          role: 'user',
          content: [
            { type: 'text', text: 'Describe this image in detail. Include the main subjects, objects, colors, composition, mood, and any notable elements.' },
            { type: 'image', data: base64, media_type: resource.mimeType }
          ]
        }]
      });

      if (visionResponse?.content) {
        rawDescription = visionResponse.content;
        console.log('[process-asset] Raw description from moondream:', rawDescription.substring(0, 150) + '...');
      } else {
        console.log('[process-asset] No content from vision model');
      }
    } catch (err) {
      console.error('[process-asset] Vision model failed:', err.message);
    }

    // ========================================
    // STAGE 2: Structure output with LLM (ollama:mistral-large-3:675b)
    // ========================================
    if (rawDescription) {
      console.log('[process-asset] === STAGE 2: STRUCTURED OUTPUT ===');
      console.log('[process-asset] Using ollama:mistral-large-3:675b with response_format...');

      try {
        const structuredResponse = await raisin.ai.completion({
          model: 'ollama:mistral-large-3:675b',
          messages: [{
            role: 'user',
            content: `Based on this image description, generate structured metadata for a content management system.

Image Description:
${rawDescription}

Generate:
1. description: A detailed description (2-3 sentences) suitable for content editors
2. alt_text: A brief accessibility description (1 sentence, max 125 characters) for screen readers
3. keywords: An array of 5-10 relevant keywords for search and categorization`
          }],
          response_format: {
            type: 'json_schema',
            json_schema: {
              name: 'image_metadata',
              schema: {
                type: 'object',
                properties: {
                  description: { type: 'string', description: 'Detailed description of the image' },
                  alt_text: { type: 'string', description: 'Brief accessibility text for screen readers' },
                  keywords: {
                    type: 'array',
                    items: { type: 'string' },
                    description: 'Search keywords'
                  }
                },
                required: ['description', 'alt_text', 'keywords']
              },
              strict: true
            }
          }
        });

        console.log('[process-asset] Structured response received:', structuredResponse?.content?.substring(0, 200));

        if (structuredResponse?.content) {
          const metadata = JSON.parse(structuredResponse.content);
          results.description = metadata.description;
          results.alt_text = metadata.alt_text;
          results.keywords = metadata.keywords;
          console.log('[process-asset] Parsed metadata:', {
            description: results.description?.substring(0, 80) + '...',
            alt_text: results.alt_text,
            keywords: results.keywords
          });
        }
      } catch (err) {
        console.error('[process-asset] Structured output failed:', err.message);
        // Fallback: use raw description if structured output fails
        results.description = rawDescription;
        results.alt_text = rawDescription.substring(0, 125);
        results.keywords = [];
        console.log('[process-asset] Using fallback from raw description');
      }
    }

    // Generate image embedding for similarity search (optional - requires local:clip model)
    console.log('[process-asset] === GENERATING EMBEDDING ===');
    try {
      const embedResponse = await raisin.ai.embed({
        model: 'local:clip',
        input: base64,
        input_type: 'image'
      });
      console.log('[process-asset] Embedding response received');

      if (embedResponse?.embedding) {
        results.embedding_generated = true;
        results.embedding_dim = embedResponse.dimensions;
        console.log('[process-asset] Embedding generated, dimensions:', embedResponse.dimensions);
      } else {
        console.log('[process-asset] No embedding in response');
      }
    } catch (err) {
      // CLIP embedding is optional - log warning and continue
      if (err.message?.includes('not found')) {
        console.warn('[process-asset] CLIP model not configured - skipping embedding generation. ' +
          'To enable image embeddings, add local:clip to tenant AI config.');
      } else {
        console.error('[process-asset] Failed to generate embedding:', err.message);
      }
    }

    // Create thumbnail and upload
    console.log('[process-asset] === CREATING THUMBNAIL ===');
    try {
      console.log('[process-asset] Resizing to thumbnail (maxWidth: 200, format: jpeg, quality: 80)...');
      const thumbnail = await resource.resize({ maxWidth: 200, format: 'jpeg', quality: 80 });
      console.log('[process-asset] Thumbnail resized, adding to node as ./thumbnail...');
      await node.addResource('./thumbnail', thumbnail);
      results.thumbnail_created = true;
      console.log('[process-asset] Thumbnail created and added successfully');
    } catch (err) {
      console.error('[process-asset] Failed to create thumbnail:', err.message);
      console.error('[process-asset] Thumbnail error stack:', err.stack);
    }

    // Update node properties with AI-generated metadata
    console.log('[process-asset] === UPDATING NODE PROPERTIES ===');

    // Build the _ai metadata object (will be nested under meta._ai)
    const aiMetadata = {
      processed_at: new Date().toISOString(),
      vision_model: 'local:moondream',
      structuring_model: 'ollama:mistral-large-3:675b',
    };

    if (results.description) {
      aiMetadata.description = results.description;
    }
    if (results.alt_text) {
      aiMetadata.alt_text = results.alt_text;
    }
    if (results.keywords && results.keywords.length > 0) {
      aiMetadata.keywords = results.keywords;
    }
    if (results.embedding_generated) {
      aiMetadata.embedding_dim = results.embedding_dim;
    }

    console.log('[process-asset] AI metadata to save:', JSON.stringify(aiMetadata, null, 2));
    console.log('[process-asset] Node path for UPDATE:', node.path);
    console.log('[process-asset] Workspace for UPDATE:', workspace);

    // Build root-level properties for direct access (schema-defined fields)
    const propertiesToMerge = {};

    if (results.description) {
      propertiesToMerge.description = results.description;
    }
    if (results.alt_text) {
      propertiesToMerge.alt_text = results.alt_text;
    }
    if (results.keywords && results.keywords.length > 0) {
      propertiesToMerge.keywords = results.keywords;
    }

    console.log('[process-asset] Properties to merge:', JSON.stringify(propertiesToMerge, null, 2));

    try {
      // Use JSONB merge operator (||) to update only specified properties
      // This preserves existing properties like thumbnail from addResource
      // For meta._ai, we use jsonb_set with coalesce to merge into existing meta
      const updateResult = await raisin.sql.query(`
        UPDATE ${workspace}
        SET properties = jsonb_set(
          properties || $1::jsonb,
          '{meta}',
          COALESCE(properties->'meta', '{}'::jsonb) || $2::jsonb
        )
        WHERE path = $3
      `, [JSON.stringify(propertiesToMerge), JSON.stringify({ _ai: aiMetadata }), node.path]);
      console.log('[process-asset] SQL update result:', JSON.stringify(updateResult, null, 2));
    } catch (sqlErr) {
      console.error('[process-asset] SQL update failed:', sqlErr.message);
      console.error('[process-asset] SQL error stack:', sqlErr.stack);
    }

    console.log('[process-asset] === FUNCTION COMPLETE ===');
    console.log('[process-asset] Final results:', JSON.stringify(results, null, 2));
    return results;

  } catch (err) {
    console.error('[process-asset] === FUNCTION ERROR ===');
    console.error('[process-asset] Error:', err.message);
    console.error('[process-asset] Stack:', err.stack);
    return { success: false, error: err.message };
  }
}

/**
 * Process PDF files - uses storage-key based API (no base64 overhead)
 *
 * The key improvement is using resource.processDocument() which:
 * 1. Works directly with storage keys (no getBinary() needed)
 * 2. Works transparently with filesystem or S3 storage
 * 3. Returns extracted text + page metadata + optional thumbnail
 * 4. Supports OCR for scanned PDFs
 */
async function processPdf(node, resource, workspace) {
  console.log('[process-asset] Processing PDF:', resource.name);
  console.log('[process-asset] Storage key:', resource.storageKey);

  const results = {
    success: true,
    description: null,
    alt_text: null,
    keywords: null,
    thumbnail_created: false,
  };

  try {
    // ========================================
    // STEP 1: Process PDF (storage-key based)
    // ========================================
    // This is the KEY API - no base64 overhead for the large PDF!
    // Works with both filesystem and S3 storage transparently.
    console.log('[process-asset] === PROCESSING PDF (storage-key based) ===');
    const pdfResult = await resource.processDocument({
      ocr: true,               // Auto-detect scanned pages and OCR them
      ocrLanguages: ['eng'],   // Tesseract language codes
      generateThumbnail: true, // First page as JPEG
      thumbnailWidth: 200,
    });

    console.log('[process-asset] PDF processed:', {
      pageCount: pdfResult.pageCount,
      isScanned: pdfResult.isScanned,
      ocrUsed: pdfResult.ocrUsed,
      textLength: pdfResult.text?.length || 0,
      hasThumbnail: !!pdfResult.thumbnail,
    });

    // ========================================
    // STEP 2: Add thumbnail (small, base64 OK)
    // ========================================
    if (pdfResult.thumbnail) {
      console.log('[process-asset] === ADDING THUMBNAIL ===');
      // thumbnail = { base64: "...", mimeType: "image/jpeg", name: "thumbnail.jpg" }
      await node.addResource('./thumbnail', pdfResult.thumbnail);
      results.thumbnail_created = true;
      console.log('[process-asset] Thumbnail added');
    }

    // ========================================
    // STEP 3: Generate AI content (if text extracted)
    // ========================================
    if (pdfResult.text && pdfResult.text.length > 50) {
      const textSample = pdfResult.text.slice(0, 4000);

      // Single structured call for all metadata
      console.log('[process-asset] === GENERATING STRUCTURED METADATA ===');
      console.log('[process-asset] Using ollama:mistral-large-3:675b with response_format...');

      const aiResponse = await raisin.ai.completion({
        model: 'ollama:mistral-large-3:675b',
        messages: [{
          role: 'user',
          content: `Analyze this PDF document text and generate metadata for a content management system.

Document Text:
${textSample}

Generate:
1. description: A detailed description (2-3 sentences) summarizing the document's purpose and content
2. alt_text: A brief accessibility description (1 sentence, max 125 characters) for screen readers
3. keywords: An array of 5-10 relevant keywords for search indexing`
        }],
        response_format: {
          type: 'json_schema',
          json_schema: {
            name: 'document_metadata',
            schema: {
              type: 'object',
              properties: {
                description: { type: 'string', description: 'Detailed description of the document' },
                alt_text: { type: 'string', description: 'Brief accessibility text for screen readers' },
                keywords: {
                  type: 'array',
                  items: { type: 'string' },
                  description: 'Search keywords'
                }
              },
              required: ['description', 'alt_text', 'keywords']
            },
            strict: true
          }
        }
      });

      if (!aiResponse?.content) {
        throw new Error('AI completion returned no content');
      }

      console.log('[process-asset] Structured response received:', aiResponse.content.substring(0, 200));

      const metadata = JSON.parse(aiResponse.content);
      results.description = metadata.description;
      results.alt_text = metadata.alt_text;
      results.keywords = metadata.keywords;

      console.log('[process-asset] Parsed metadata:', {
        description: results.description?.substring(0, 80) + '...',
        alt_text: results.alt_text,
        keywords: results.keywords
      });
    } else if (pdfResult.isScanned && !pdfResult.ocrUsed) {
      console.log('[process-asset] PDF is scanned but OCR not available');
      results.description = 'Scanned PDF document - enable OCR for text extraction.';
    }

    // ========================================
    // STEP 4: Update node properties
    // ========================================
    console.log('[process-asset] === UPDATING NODE PROPERTIES ===');

    // Root-level schema-defined properties
    const propertiesToMerge = {
      description: results.description,
      alt_text: results.alt_text,
      keywords: results.keywords,
    };

    // Meta object with document info and AI-generated content
    const metaToMerge = {
      document: {
        page_count: pdfResult.pageCount,
        is_scanned: pdfResult.isScanned,
        ocr_used: pdfResult.ocrUsed,
        extraction_method: pdfResult.extractionMethod,
        mime_type: 'application/pdf',
      },
      _ai: {
        processed_at: new Date().toISOString(),
        model: 'ollama:mistral-large-3:675b',
        description: results.description,
        alt_text: results.alt_text,
        keywords: results.keywords,
        // Full text for search indexing (truncated to 100KB)
        extracted_text: pdfResult.text?.slice(0, 100000),
      },
    };

    // Use jsonb_set with coalesce to merge into existing meta without overwriting
    await raisin.sql.query(`
      UPDATE ${workspace}
      SET properties = jsonb_set(
        properties || $1::jsonb,
        '{meta}',
        COALESCE(properties->'meta', '{}'::jsonb) || $2::jsonb
      )
      WHERE path = $3
    `, [JSON.stringify(propertiesToMerge), JSON.stringify(metaToMerge), node.path]);
    console.log('[process-asset] Properties updated');

    console.log('[process-asset] === PDF PROCESSING COMPLETE ===');
    return { success: true, ...results };

  } catch (err) {
    console.error('[process-asset] PDF error:', err.message);
    return { success: false, error: err.message };
  }
}

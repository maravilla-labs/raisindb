/**
 * Process Asset
 *
 * AI-powered asset processing function that demonstrates the Resource API.
 * Generates alt-text, descriptions, keywords, and thumbnails for uploaded images.
 *
 * This function is triggered when a raisin:Asset is updated and has a
 * storage_key (meaning the file upload is complete).
 */
async function updateProgress(workspace, nodePath, progress, status) {
  await raisin.sql.query(`
    UPDATE ${workspace}
    SET properties = jsonb_set(
      properties,
      '{meta,processing}',
      $1::jsonb
    )
    WHERE path = $2
  `, [JSON.stringify({ progress, status }), nodePath]);
}

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

  // Create thumbnail immediately so the UI can show a preview while AI processes
  await updateProgress(workspace, node.path, 10, 'Generating thumbnail...');
  console.log('[process-asset] === CREATING THUMBNAIL (early) ===');
  let thumbnailCreated = false;
  try {
    console.log('[process-asset] Resizing to thumbnail (maxWidth: 200, format: jpeg, quality: 80)...');
    const thumbnail = await resource.resize({ maxWidth: 200, format: 'jpeg', quality: 80 });
    console.log('[process-asset] Thumbnail resized, adding to node as ./thumbnail...');
    await node.addResource('./thumbnail', thumbnail);
    thumbnailCreated = true;
    console.log('[process-asset] Thumbnail created and saved to node');
  } catch (err) {
    console.error('[process-asset] Failed to create thumbnail:', err.message);
    console.error('[process-asset] Thumbnail error stack:', err.stack);
  }

  if (thumbnailCreated) {
    await updateProgress(workspace, node.path, 20, 'Thumbnail ready');
  }

  const results = {
    success: true,
    title: null,
    description: null,
    alt_text: null,
    keywords: null,
    thumbnail_created: thumbnailCreated,
    embedding_generated: false,
  };

  try {
    // Resize to 512px for vision model processing
    await updateProgress(workspace, node.path, 30, 'Preparing image...');
    console.log('[process-asset] Resizing image for processing (maxWidth: 512)...');
    const resized = await resource.resize({ maxWidth: 512 });
    console.log('[process-asset] Resize complete, getting binary data...');
    const base64 = await resized.getBinary();
    console.log('[process-asset] Binary data retrieved, base64 length:', base64?.length || 0);

    // ========================================
    // SINGLE-STAGE: Vision + metadata with llava:7b
    // ========================================
    await updateProgress(workspace, node.path, 40, 'Analyzing image...');
    console.log('[process-asset] === VISION + METADATA (ollama:llava:7b) ===');

    try {
      const visionResponse = await raisin.ai.completion({
        model: 'ollama:llava:7b',
        messages: [{
          role: 'user',
          content: [
            { type: 'text', text: `Analyze this image and return a JSON object. First determine the content type, then provide appropriate metadata.

STEP 1: Identify content_type (one of: "photo", "invoice", "receipt", "document", "screenshot", "artwork", "diagram")

STEP 2: Based on content_type, include these fields:

FOR "photo", "artwork", "screenshot":
- title: Short descriptive title (3-7 words)
- description: What the image shows (2-3 sentences)
- alt_text: Accessibility text (max 125 chars)
- keywords: Array of 8-12 classification terms. ONLY use:
  * Concrete objects visible (e.g., "car", "tree", "laptop", "dog")
  * Object categories (e.g., "vehicle", "animal", "furniture", "food")
  * Scene classifications (e.g., "outdoor", "indoor", "urban", "nature")
  * Activity classifications (e.g., "sports", "cooking", "meeting")
  * NO adjectives, NO abstract concepts, NO subjective descriptions

FOR "invoice":
- title: "Invoice [number] - [vendor]" or best attempt
- vendor: Company/person who issued it
- invoice_number: Invoice/reference number if visible
- date: Date on invoice (YYYY-MM-DD if possible)
- due_date: Due date if visible
- total: Total amount with currency
- line_items: Array of items/services (brief descriptions)
- keywords: Array with vendor name, "invoice", product/service categories mentioned

FOR "receipt":
- title: "Receipt - [merchant]" or best attempt
- merchant: Store/business name
- date: Transaction date
- total: Total amount with currency
- items: Array of purchased items (brief)
- payment_method: If visible (cash, card, etc.)
- keywords: Array with merchant name, "receipt", store type, product categories purchased

FOR "document":
- title: Document title or type
- document_type: Letter, contract, form, report, etc.
- date: Document date if visible
- author_or_org: Who created/sent it
- summary: Key content summary (2-3 sentences of actual content, not description)
- keywords: Array with document_type, organization names, key topic classifications from content

FOR "diagram":
- title: What the diagram shows
- diagram_type: Flowchart, org chart, architecture, etc.
- description: What it illustrates
- labels: Key labels/text from the diagram
- keywords: Array with diagram_type, domain classification (e.g., "software", "business", "engineering"), key entities shown

ALWAYS include: content_type, title, keywords, alt_text

Return ONLY valid JSON, no other text.` },
            { type: 'image', data: base64, media_type: resource.mimeType }
          ]
        }]
      });

      console.log('[process-asset] Raw llava response:', JSON.stringify(visionResponse, null, 2));

      if (visionResponse?.content) {
        // Extract JSON from the response (handle possible markdown fences)
        let jsonStr = visionResponse.content.trim();
        const fenceMatch = jsonStr.match(/```(?:json)?\s*([\s\S]*?)```/);
        if (fenceMatch) {
          jsonStr = fenceMatch[1].trim();
        }

        const metadata = JSON.parse(jsonStr);
        results.content_type = metadata.content_type || 'photo';
        results.title = metadata.title || null;
        results.description = metadata.description || metadata.summary || null;
        results.alt_text = metadata.alt_text || null;
        results.keywords = metadata.keywords || [];

        // Document-specific fields (invoice, receipt, document, diagram)
        if (['invoice', 'receipt', 'document', 'diagram'].includes(metadata.content_type)) {
          results.extracted_data = {
            vendor: metadata.vendor,
            merchant: metadata.merchant,
            invoice_number: metadata.invoice_number,
            date: metadata.date,
            due_date: metadata.due_date,
            total: metadata.total,
            line_items: metadata.line_items || metadata.items,
            document_type: metadata.document_type,
            author_or_org: metadata.author_or_org,
            summary: metadata.summary,
            payment_method: metadata.payment_method,
            labels: metadata.labels,
            diagram_type: metadata.diagram_type
          };
          // Remove undefined values
          results.extracted_data = Object.fromEntries(
            Object.entries(results.extracted_data).filter(([_, v]) => v !== undefined)
          );
        }

        console.log('[process-asset] Parsed metadata:', {
          content_type: results.content_type,
          title: results.title,
          description: results.description?.substring(0, 80) + '...',
          alt_text: results.alt_text,
          keywords: results.keywords,
          extracted_data: results.extracted_data
        });
      } else {
        console.log('[process-asset] No content in llava response');
      }
    } catch (err) {
      console.error('[process-asset] llava:7b failed:', err.message);
    }

    /*
     * OLD TWO-STAGE APPROACH (kept for reference):
     * Stage 1 used local:moondream for raw image description,
     * Stage 2 used ollama:mistral-large-3:675b to structure the output.
     *
     * ========================================
     * STAGE 1: Get raw description from vision model (local:moondream)
     * ========================================
     * await updateProgress(workspace, node.path, 40, 'Analyzing image...');
     * const visionResponse = await raisin.ai.completion({
     *   model: 'local:moondream',
     *   messages: [{ role: 'user', content: [
     *     { type: 'text', text: 'Describe this image in detail...' },
     *     { type: 'image', data: base64, media_type: resource.mimeType }
     *   ]}]
     * });
     * rawDescription = visionResponse?.content;
     *
     * ========================================
     * STAGE 2: Structure output with LLM (ollama:mistral-large-3:675b)
     * ========================================
     * const structuredResponse = await raisin.ai.completion({
     *   model: 'ollama:mistral-large-3:675b',
     *   messages: [{ role: 'user', content: `Based on this image description...` }],
     *   response_format: { type: 'json_schema', json_schema: { ... } }
     * });
     * const metadata = JSON.parse(structuredResponse.content);
     */

    // Generate image embedding for similarity search (optional - requires local:clip model)
    await updateProgress(workspace, node.path, 80, 'Creating embedding...');
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

    // Update node properties with AI-generated metadata
    console.log('[process-asset] === UPDATING NODE PROPERTIES ===');

    // Build the _ai metadata object (will be nested under meta._ai)
    const aiMetadata = {
      processed_at: new Date().toISOString(),
      model: 'ollama:llava:7b',
      content_type: results.content_type || 'photo',
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

    if (results.content_type) {
      propertiesToMerge.content_type = results.content_type;
    }
    if (results.title) {
      propertiesToMerge.title = results.title;
    }
    if (results.description) {
      propertiesToMerge.description = results.description;
    }
    if (results.alt_text) {
      propertiesToMerge.alt_text = results.alt_text;
    }
    if (results.keywords && results.keywords.length > 0) {
      propertiesToMerge.keywords = results.keywords;
    }
    if (results.extracted_data && Object.keys(results.extracted_data).length > 0) {
      propertiesToMerge.extracted_data = results.extracted_data;
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

    await updateProgress(workspace, node.path, 100, 'Complete');
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

  await updateProgress(workspace, node.path, 10, 'Processing PDF...');

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
    await updateProgress(workspace, node.path, 20, 'Generating thumbnail...');

    if (pdfResult.thumbnail) {
      console.log('[process-asset] === ADDING THUMBNAIL ===');
      // thumbnail = { base64: "...", mimeType: "image/jpeg", name: "thumbnail.jpg" }
      await node.addResource('./thumbnail', pdfResult.thumbnail);
      results.thumbnail_created = true;
      console.log('[process-asset] Thumbnail added');
      await updateProgress(workspace, node.path, 30, 'Thumbnail ready');
    }

    // ========================================
    // STEP 3: Generate AI content (if text extracted)
    // ========================================
    if (pdfResult.text && pdfResult.text.length > 50) {
      await updateProgress(workspace, node.path, 40, 'Analyzing document...');

      const textSample = pdfResult.text.slice(0, 4000);

      // Single structured call for all metadata
      console.log('[process-asset] === GENERATING STRUCTURED METADATA ===');
      console.log('[process-asset] Using ollama:llama3:8b-instruct-q4_K_M with response_format...');

      const aiResponse = await raisin.ai.completion({
        model: 'ollama:llama3:8b-instruct-q4_K_M',
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
    await updateProgress(workspace, node.path, 80, 'Finalizing...');
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
        model: 'ollama:llama3:8b-instruct-q4_K_M',
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

    await updateProgress(workspace, node.path, 100, 'Complete');
    console.log('[process-asset] === PDF PROCESSING COMPLETE ===');
    return { success: true, ...results };

  } catch (err) {
    console.error('[process-asset] PDF error:', err.message);
    return { success: false, error: err.message };
  }
}

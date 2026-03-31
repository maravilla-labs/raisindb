# raisin-ai

AI provider integration and content processing for RaisinDB.

## Overview

This crate provides a unified abstraction layer for multiple AI providers, secure API key management, and content processing pipelines for embeddings and document extraction.

## Supported Providers

- OpenAI (GPT-4, GPT-3.5, embeddings)
- Anthropic (Claude models)
- Google Gemini (1.5, 2.0)
- Azure OpenAI
- AWS Bedrock
- Ollama (local models)
- Groq
- OpenRouter
- Local inference via Candle

## Modules

| Module | Description |
|--------|-------------|
| `config` | Tenant-level AI configuration with multi-provider support |
| `crypto` | API key encryption using AES-256-GCM |
| `storage` | Abstract trait for persisting tenant configurations |
| `provider` | Core `AIProviderTrait` defining the provider interface |
| `providers` | Concrete provider implementations |
| `types` | Unified message, completion, and tool-calling types |
| `pdf` | PDF text extraction (native, markdown, OCR) |
| `chunking` | Text splitting for embeddings with token-aware overlap |
| `rules` | Processing rule system for content handling |
| `candle` | Local inference with CLIP, BLIP, Moondream models |
| `huggingface` | HuggingFace Hub model management |
| `model_cache` | In-memory model metadata caching |
| `validation` | JSON schema validation for structured outputs |

## Feature Flags

| Feature | Description |
|---------|-------------|
| `default` | Includes `tiktoken` for token counting |
| `tiktoken` | Token counting for text chunking |
| `huggingface` | HuggingFace Hub model downloads |
| `pdf` | PDF text extraction via pdf-extract |
| `pdf-markdown` | PDF to markdown conversion |
| `ocr` | Tesseract OCR for scanned PDFs |
| `candle` | Local AI inference (CLIP, BLIP, Moondream) |

## Internal Dependencies

- `raisin-storage` - Storage abstraction for PDF processing

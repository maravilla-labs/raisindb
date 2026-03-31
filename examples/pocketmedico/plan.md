# Pocket Medico - Implementation Plan

## Executive Summary

Pocket Medico is a hybrid AI + human-powered medical transcription platform for doctors. The MVP focuses on proving that users can register, login, upload audio/notes, and receive transcribed documents.

---

## Part 1: Core Functionality (MVP)

Based on the landing page description, the MVP delivers:

### User Workflow (3 Steps)

1. **Upload**: Doctor uploads audio files or handwritten notes
2. **Transcription**: AI processes (Light) or human reviews (Pro)
3. **Download**: Doctor downloads professional medical documents

### Service Tiers

| Tier | Name | Description |
|------|------|-------------|
| Light | AI-Powered | Fully automated AI transcription |
| Pro | Manual Review | AI + human expert verification |

### User Roles

| Role | German | Description | Permissions |
|------|--------|-------------|-------------|
| Customer | Arzt (Doctor) | Creates orders, uploads files, downloads transcripts | Create orders, view own orders, download |
| Nurse | Transkriptor | Reviews/edits AI transcriptions | View assigned orders, edit transcripts, approve |

---

## Part 2: RaisinDB Node Types

### Core Node Types

#### 1. `medico:User`

Extends the authentication system with role-specific data.

```yaml
name: medico:User
title: Pocket Medico User
description: User profile for doctors and nurses
icon: user

properties:
  - name: role
    title: Role
    type: String
    required: true
    ui:
      widget: select
      options:
        - label: Customer (Doctor)
          value: customer
        - label: Nurse (Transcriber)
          value: nurse

  - name: practice_name
    title: Practice Name
    type: String
    description: Doctor's practice name (for customers)

  - name: specialization
    title: Specialization
    type: String
    description: Medical specialization

  - name: phone
    title: Phone
    type: String

  - name: verified
    title: Verified
    type: Boolean
    default: false
```

#### 2. `medico:TranscriptionOrder`

The main order entity representing a transcription request.

```yaml
name: medico:TranscriptionOrder
title: Transcription Order
description: A medical transcription order
icon: file-audio
color: "#0891B2"

allowed_children:
  - medico:AudioFile
  - medico:NoteImage
  - medico:Transcript

properties:
  - name: order_number
    title: Order Number
    type: String
    required: true
    unique: true
    readonly: true

  - name: customer_ref
    title: Customer
    type: Reference
    required: true
    description: Reference to the ordering doctor

  - name: tier
    title: Service Tier
    type: String
    required: true
    default: light
    ui:
      widget: select
      options:
        - label: Light (AI Only)
          value: light
        - label: Pro (Human Review)
          value: pro

  - name: status
    title: Status
    type: String
    default: pending
    ui:
      widget: select
      options:
        - label: Pending Upload
          value: pending
        - label: Processing
          value: processing
        - label: AI Complete
          value: ai_complete
        - label: In Review
          value: in_review
        - label: Ready
          value: ready
        - label: Downloaded
          value: downloaded
        - label: Error
          value: error

  - name: assigned_nurse_ref
    title: Assigned Nurse
    type: Reference
    description: Nurse assigned for Pro tier review

  - name: priority
    title: Priority
    type: String
    default: normal
    ui:
      widget: select
      options:
        - label: Normal
          value: normal
        - label: Urgent
          value: urgent

  - name: notes
    title: Notes
    type: String
    ui:
      widget: textarea

  - name: patient_initials
    title: Patient Initials
    type: String
    description: For identification without full name

  - name: document_type
    title: Document Type
    type: String
    ui:
      widget: select
      options:
        - label: Medical Report
          value: medical_report
        - label: Discharge Summary
          value: discharge_summary
        - label: Consultation Note
          value: consultation_note
        - label: Prescription
          value: prescription
        - label: Other
          value: other

  - name: total_duration_seconds
    title: Total Audio Duration
    type: Number
    readonly: true

  - name: estimated_completion
    title: Estimated Completion
    type: Date

  - name: completed_at
    title: Completed At
    type: Date
    readonly: true

  - name: ai_confidence_score
    title: AI Confidence Score
    type: Number
    readonly: true
    description: 0-100 confidence score from AI

  - name: metadata
    title: Metadata
    type: Object
```

#### 3. `medico:AudioFile`

Uploaded audio recording for transcription.

```yaml
name: medico:AudioFile
title: Audio File
description: Audio recording for transcription
icon: mic

properties:
  - name: file
    title: Audio File
    type: Resource
    required: true

  - name: duration_seconds
    title: Duration (seconds)
    type: Number
    readonly: true

  - name: format
    title: Format
    type: String
    readonly: true

  - name: size_bytes
    title: Size (bytes)
    type: Number
    readonly: true

  - name: transcription_status
    title: Transcription Status
    type: String
    default: pending
```

#### 4. `medico:NoteImage`

Uploaded handwritten note/image for transcription.

```yaml
name: medico:NoteImage
title: Note Image
description: Handwritten note or image for transcription
icon: image

properties:
  - name: file
    title: Image File
    type: Resource
    required: true

  - name: ocr_status
    title: OCR Status
    type: String
    default: pending

  - name: ocr_text
    title: OCR Text
    type: String
    readonly: true
    ui:
      widget: textarea
```

#### 5. `medico:Transcript`

The resulting transcription document.

```yaml
name: medico:Transcript
title: Transcript
description: Transcribed medical document
icon: file-text
color: "#10B981"

publishable: true

properties:
  - name: content
    title: Content
    type: String
    required: true
    fulltext: true
    ui:
      widget: richtext

  - name: source_type
    title: Source Type
    type: String
    ui:
      widget: select
      options:
        - label: Audio
          value: audio
        - label: Handwritten Notes
          value: notes
        - label: Mixed
          value: mixed

  - name: version
    title: Version
    type: Number
    default: 1

  - name: ai_generated
    title: AI Generated
    type: Boolean
    default: true

  - name: human_reviewed
    title: Human Reviewed
    type: Boolean
    default: false

  - name: reviewer_ref
    title: Reviewed By
    type: Reference

  - name: reviewed_at
    title: Reviewed At
    type: Date

  - name: review_notes
    title: Review Notes
    type: String
    ui:
      widget: textarea

  - name: export_format
    title: Export Format
    type: String
    default: pdf
    ui:
      widget: select
      options:
        - label: PDF
          value: pdf
        - label: DOCX
          value: docx
        - label: Plain Text
          value: txt
```

---

## Part 3: Workspace Structure

### `medico` Workspace

```yaml
name: medico
title: Pocket Medico
description: Medical transcription workspace
icon: stethoscope
color: "#0891B2"

allowed_node_types:
  - raisin:Folder
  - medico:User
  - medico:TranscriptionOrder
  - medico:AudioFile
  - medico:NoteImage
  - medico:Transcript

allowed_root_node_types:
  - raisin:Folder

root_structure:
  - name: customers
    node_type: raisin:Folder
    title: Customers
    description: Customer (doctor) profiles and orders

  - name: nurses
    node_type: raisin:Folder
    title: Nurses
    description: Nurse profiles

  - name: orders
    node_type: raisin:Folder
    title: All Orders
    description: Central order management

  - name: templates
    node_type: raisin:Folder
    title: Templates
    description: Document templates

  - name: config
    node_type: raisin:Folder
    title: Configuration
    description: System configuration
```

### User Folder Structure

Each customer gets a personal folder:

```
/customers/{user-id}/
├── profile/             # User profile data
├── orders/              # User's transcription orders
│   └── {order-id}/
│       ├── audio/       # Uploaded audio files
│       ├── notes/       # Uploaded note images
│       └── transcripts/ # Generated transcripts
└── settings/            # User preferences
```

---

## Part 4: Triggers and Job Queue

### Trigger: On Audio Upload

**Purpose**: Start AI transcription when audio is uploaded.

```yaml
node_type: raisin:Trigger
properties:
  name: on-audio-upload
  title: Process Audio Upload
  description: Triggers AI transcription when audio file is uploaded
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - medico:AudioFile

  priority: 10
  max_retries: 3
  function_path: /functions/handlers/process-audio-upload
```

### Trigger: On AI Transcription Complete

**Purpose**: Handle completed AI transcription, route to review or mark ready.

```yaml
node_type: raisin:Trigger
properties:
  name: on-ai-complete
  title: Handle AI Transcription Complete
  description: Routes order based on tier (Light=ready, Pro=review)
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Updated

  filters:
    node_types:
      - medico:TranscriptionOrder
    property_filters:
      status: ai_complete

  priority: 10
  max_retries: 3
  function_path: /functions/handlers/route-completed-transcription
```

### Trigger: On Review Approved

**Purpose**: Mark Pro tier orders as ready after nurse approval.

```yaml
node_type: raisin:Trigger
properties:
  name: on-review-approved
  title: Handle Review Approval
  description: Marks order as ready after nurse approves transcript
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Updated

  filters:
    node_types:
      - medico:Transcript
    property_filters:
      human_reviewed: true

  priority: 10
  max_retries: 3
  function_path: /functions/handlers/finalize-order
```

### Job Handlers

Using unified job queue pattern from CLAUDE.md:

```typescript
// Register transcription job type
JobRegistry.register_job({
  job_type: 'transcribe_audio',
  handler: async (payload) => {
    const { orderId, audioFileId } = payload;

    // 1. Download audio from storage
    // 2. Send to AI transcription service
    // 3. Create Transcript node with result
    // 4. Update order status
  },
  max_retries: 3
});

// Queue a transcription job
await JobDataStore.put({
  job_type: 'transcribe_audio',
  payload: {
    orderId: order.id,
    audioFileId: audioFile.id
  },
  status: 'pending'
});
```

---

## Part 5: Authentication & Authorization

### Integration with AUTH_CONCEPT.md

Following the pluggable authentication architecture:

#### Registration Flow

1. User visits `/register`
2. Selects role: Customer (Doctor) or Nurse
3. For MVP: Use Local Strategy (email/password)
4. Creates Identity in `raisin:system` workspace
5. Creates `medico:User` node in `medico` workspace
6. For nurses: Requires admin approval (status: pending)

#### Login Flow

1. Use Local Strategy authentication
2. JWT includes:
   - `identity_id`
   - `role` (customer/nurse)
   - `workspace_permissions` for `medico` workspace

#### Role-Based Access Control

```yaml
# Customer permissions
customer:
  can_create:
    - medico:TranscriptionOrder
    - medico:AudioFile
    - medico:NoteImage
  can_read:
    - own orders only
  can_update:
    - own orders (before submission)
  can_delete:
    - own drafts only

# Nurse permissions
nurse:
  can_read:
    - assigned orders
    - all orders (if admin)
  can_update:
    - transcripts (edit, approve)
    - order status
  cannot:
    - create orders
    - delete orders
```

### Simplified MVP Auth

For MVP, use simplified authentication:

1. **Registration Page** (`/register`)
   - Email, password, role selection
   - Nurses require email verification

2. **Login Page** (`/login`)
   - Email/password
   - Redirect based on role

3. **Session Management**
   - JWT tokens stored in HTTP-only cookies
   - 24h session duration
   - Refresh token support

---

## Part 6: SvelteKit App Architecture

### Route Structure

```
src/routes/
├── +layout.svelte          # Root layout with navigation
├── +page.svelte            # Landing page
├── (auth)/
│   ├── login/+page.svelte
│   ├── register/+page.svelte
│   └── logout/+page.server.ts
├── (customer)/
│   ├── dashboard/+page.svelte
│   ├── orders/
│   │   ├── +page.svelte       # Order list
│   │   ├── new/+page.svelte   # Create order
│   │   └── [id]/
│   │       ├── +page.svelte   # Order detail
│   │       └── download/+page.server.ts
│   └── settings/+page.svelte
├── (nurse)/
│   ├── queue/+page.svelte     # Review queue
│   └── review/[id]/+page.svelte
└── api/
    ├── auth/+server.ts
    ├── orders/+server.ts
    └── upload/+server.ts
```

### Key Components

1. **AuthGuard**: Route protection based on role
2. **FileUploader**: Drag-drop audio/image upload
3. **AudioPlayer**: Preview uploaded audio
4. **TranscriptEditor**: Rich text editor for nurses
5. **OrderCard**: Order summary card
6. **StatusBadge**: Order status indicator

### Database Connection

Following news-feed pattern with RaisinDB PostgreSQL protocol:

```typescript
// src/lib/server/db.ts
import { Pool } from 'pg';

const pool = new Pool({
  connectionString: process.env.DATABASE_URL
});

export async function query<T>(sql: string, params?: any[]): Promise<T[]> {
  const client = await pool.connect();
  try {
    const result = await client.query(sql, params);
    return result.rows as T[];
  } finally {
    client.release();
  }
}
```

---

## Part 7: MVP Implementation Phases

### Phase 1: Foundation

- [ ] Set up RaisinDB package with node types
- [ ] Set up SvelteKit app with TailwindCSS
- [ ] Implement database connection
- [ ] Create basic layouts and navigation

### Phase 2: Authentication

- [ ] Registration page (customer/nurse selection)
- [ ] Login page
- [ ] Session management
- [ ] Route guards

### Phase 3: Customer Flow

- [ ] Customer dashboard
- [ ] Create new order form
- [ ] File upload (audio/images)
- [ ] Order list view
- [ ] Order detail view
- [ ] Download transcript

### Phase 4: AI Integration

- [ ] Trigger: on audio upload
- [ ] Job: transcribe audio (mock AI for MVP)
- [ ] Create transcript from AI result
- [ ] Update order status

### Phase 5: Nurse Review (Pro Tier)

- [ ] Nurse queue view
- [ ] Transcript editor
- [ ] Approve/reject workflow
- [ ] Review history

### Phase 6: Polish

- [ ] Email notifications (via job queue)
- [ ] Order status timeline
- [ ] Export formats (PDF, DOCX)
- [ ] Error handling

---

## Part 8: Data Flow Diagrams

### Order Creation Flow

```
Customer                    App                      RaisinDB
   │                         │                          │
   │  Create Order Form      │                          │
   │────────────────────────>│                          │
   │                         │  INSERT order            │
   │                         │─────────────────────────>│
   │                         │                          │
   │  Upload Audio           │                          │
   │────────────────────────>│                          │
   │                         │  INSERT audio_file       │
   │                         │─────────────────────────>│
   │                         │                          │
   │                         │  TRIGGER: on-audio-upload│
   │                         │<─────────────────────────│
   │                         │                          │
   │                         │  JOB: transcribe_audio   │
   │                         │─────────────────────────>│
   │                         │                          │
   │  Order Submitted!       │                          │
   │<────────────────────────│                          │
```

### Transcription Processing Flow

```
Job Queue                  AI Service               RaisinDB
   │                           │                       │
   │  Process Job              │                       │
   │──────────────────────────>│                       │
   │                           │                       │
   │                           │  Transcribe Audio     │
   │                           │───┐                   │
   │                           │<──┘                   │
   │                           │                       │
   │  AI Result                │                       │
   │<──────────────────────────│                       │
   │                           │                       │
   │  INSERT transcript        │                       │
   │──────────────────────────────────────────────────>│
   │                           │                       │
   │  UPDATE order.status      │                       │
   │──────────────────────────────────────────────────>│
   │                           │                       │
   │  TRIGGER: on-ai-complete  │                       │
   │<──────────────────────────────────────────────────│
```

---

## Part 9: Security Considerations

### Data Protection (GDPR/HIPAA Considerations)

1. **Patient Data**: Only initials, no full names
2. **Audio Storage**: Encrypted at rest
3. **Access Logs**: Full audit trail
4. **Data Retention**: Configurable retention policies
5. **Right to Delete**: Support for data erasure

### File Upload Security

1. **Type Validation**: Only audio/image formats
2. **Size Limits**: Max 100MB per file
3. **Virus Scanning**: Integration point for scanning
4. **Secure URLs**: Time-limited signed URLs

---

## Part 10: Future Enhancements (Post-MVP)

1. **Real-time Transcription**: Live audio streaming
2. **Mobile App**: React Native companion app
3. **Practice Integration**: API for practice management software
4. **Multi-language**: Support for multiple languages
5. **Templates**: Customizable document templates
6. **Analytics**: Usage and quality metrics dashboard
7. **Billing**: Subscription and pay-per-use billing
8. **Team Management**: Multiple users per practice

---

## Appendix A: Sample SQL Queries

### Create Order

```sql
BEGIN;

INSERT INTO medico (path, node_type, name, properties)
VALUES (
  '/customers/{user-id}/orders/{order-id}',
  'medico:TranscriptionOrder',
  'Order #2024-001',
  '{
    "order_number": "2024-001",
    "customer_ref": {"raisin:ref": "user-id", "raisin:workspace": "medico"},
    "tier": "light",
    "status": "pending",
    "document_type": "medical_report"
  }'
);

COMMIT WITH MESSAGE 'Created transcription order';
```

### Get Customer Orders

```sql
SELECT
  id,
  path,
  name,
  properties ->> 'order_number' AS order_number,
  properties ->> 'status' AS status,
  properties ->> 'tier' AS tier,
  created_at
FROM medico
WHERE DESCENDANT_OF('/customers/{user-id}/orders')
  AND node_type = 'medico:TranscriptionOrder'
ORDER BY created_at DESC;
```

### Get Nurse Review Queue

```sql
SELECT
  o.id,
  o.path,
  o.properties ->> 'order_number' AS order_number,
  o.properties ->> 'tier' AS tier,
  o.created_at,
  c.properties ->> 'practice_name' AS practice_name
FROM medico o
JOIN medico c ON c.id = (o.properties -> 'customer_ref' ->> 'raisin:ref')::uuid
WHERE o.node_type = 'medico:TranscriptionOrder'
  AND o.properties ->> 'tier' = 'pro'
  AND o.properties ->> 'status' = 'ai_complete'
ORDER BY o.created_at ASC;
```

---

## Appendix B: Environment Variables

```env
# Database
DATABASE_URL=postgresql://user:pass@localhost:5432/raisindb

# Authentication
JWT_SECRET=your-secret-key
SESSION_DURATION_HOURS=24

# AI Service (placeholder for MVP)
AI_SERVICE_URL=https://api.example.com/transcribe
AI_SERVICE_KEY=your-api-key

# File Storage
STORAGE_BUCKET=pocketmedico-uploads
STORAGE_REGION=eu-central-1

# Email (for notifications)
SMTP_HOST=smtp.example.com
SMTP_PORT=587
SMTP_USER=noreply@pocketmedico.de
SMTP_PASS=your-password
```

---

## Appendix C: API Endpoints (REST-style)

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | /api/auth/register | Register new user |
| POST | /api/auth/login | Login |
| POST | /api/auth/logout | Logout |
| GET | /api/orders | List orders (filtered by role) |
| POST | /api/orders | Create new order |
| GET | /api/orders/:id | Get order details |
| PATCH | /api/orders/:id | Update order |
| POST | /api/orders/:id/upload | Upload file to order |
| GET | /api/orders/:id/transcript | Get transcript |
| PATCH | /api/orders/:id/transcript | Update transcript (nurse) |
| POST | /api/orders/:id/approve | Approve transcript (nurse) |
| GET | /api/orders/:id/download | Download transcript |

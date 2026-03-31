<script lang="ts">
	import { goto } from '$app/navigation';
	import { ArrowLeft, Send } from 'lucide-svelte';
	import { Button, Input, Select } from '$lib/components/shared';
	import { AppHeader } from '$lib/components/layout';
	import { FileUploadZone, FileList } from '$lib/components/orders';
	import {
		createOrder,
		addFileToOrder,
		removeFileFromOrder,
		submitOrder,
		type ServiceTier,
		type DocumentType,
		type UploadedFile
	} from '$lib/stores/orders';
	import { toasts } from '$lib/stores/toast';

	// Form state
	let tier = $state<ServiceTier>('light');
	let documentType = $state<DocumentType>('medical_report');
	let patientInitials = $state('');
	let notes = $state('');
	let files = $state<UploadedFile[]>([]);
	let isSubmitting = $state(false);

	// Options
	const tierOptions = [
		{ value: 'light', label: 'Light - AI Only' },
		{ value: 'pro', label: 'Pro - AI + Nurse Review' }
	];

	const documentTypeOptions = [
		{ value: 'medical_report', label: 'Medical Report' },
		{ value: 'discharge_summary', label: 'Discharge Summary' },
		{ value: 'consultation_note', label: 'Consultation Note' },
		{ value: 'prescription', label: 'Prescription' },
		{ value: 'other', label: 'Other' }
	];

	// Validation
	let isValid = $derived(patientInitials.trim().length > 0 && files.length > 0);

	// Handle file uploads
	function handleFiles(newFiles: File[]) {
		const uploadedFiles: UploadedFile[] = newFiles.map((file) => ({
			id: crypto.randomUUID(),
			name: file.name,
			type: file.type.startsWith('audio/') ? 'audio' : 'image',
			size: file.size,
			url: URL.createObjectURL(file)
		}));
		files = [...files, ...uploadedFiles];
	}

	// Handle file removal
	function handleRemoveFile(fileId: string) {
		const file = files.find((f) => f.id === fileId);
		if (file) {
			URL.revokeObjectURL(file.url);
		}
		files = files.filter((f) => f.id !== fileId);
	}

	// Submit order
	async function handleSubmit() {
		if (!isValid || isSubmitting) return;

		isSubmitting = true;

		try {
			// Create the order
			const order = createOrder({
				tier,
				documentType,
				patientInitials: patientInitials.trim(),
				notes: notes.trim() || undefined
			});

			// Add files to the order
			for (const file of files) {
				addFileToOrder(order.id, file);
			}

			// Submit the order for processing
			submitOrder(order.id);

			toasts.success('Order submitted successfully!');
			goto(`/orders/${order.id}`);
		} catch (error) {
			toasts.error('Failed to create order. Please try again.');
			isSubmitting = false;
		}
	}
</script>

<svelte:head>
	<title>New Order - Pocket Medico</title>
</svelte:head>

<AppHeader title="New Order">
	<a href="/orders">
		<Button variant="ghost">
			<ArrowLeft class="h-4 w-4" />
			Back to Orders
		</Button>
	</a>
</AppHeader>

<div class="mx-auto max-w-2xl p-6">
	<div class="rounded-xl border border-gray-200 bg-white p-6">
		<h2 class="mb-6 text-lg font-semibold text-gray-900">Order Details</h2>

		<form onsubmit={(e) => { e.preventDefault(); handleSubmit(); }} class="space-y-6">
			<!-- Service Tier -->
			<Select
				label="Service Tier"
				options={tierOptions}
				bind:value={tier}
			/>

			<!-- Document Type -->
			<Select
				label="Document Type"
				options={documentTypeOptions}
				bind:value={documentType}
			/>

			<!-- Patient Initials -->
			<Input
				label="Patient Initials"
				placeholder="e.g., M.K."
				bind:value={patientInitials}
				required
			/>

			<!-- Notes -->
			<div class="space-y-1">
				<label for="notes" class="block text-sm font-medium text-gray-700">
					Notes (optional)
				</label>
				<textarea
					id="notes"
					bind:value={notes}
					placeholder="Any additional notes or instructions..."
					rows="3"
					class="block w-full rounded-lg border border-gray-300 px-3 py-2 text-sm transition-colors
						placeholder:text-gray-400
						focus:border-blue-500 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-0"
				></textarea>
			</div>

			<!-- File Upload Section -->
			<div class="space-y-3">
				<label class="block text-sm font-medium text-gray-700">
					Upload Files
				</label>
				<FileUploadZone onfiles={handleFiles} />
				<FileList {files} onremove={handleRemoveFile} />
			</div>

			<!-- Submit Button -->
			<div class="flex justify-end pt-4">
				<Button
					type="submit"
					disabled={!isValid}
					loading={isSubmitting}
					size="lg"
				>
					<Send class="h-4 w-4" />
					Submit Order
				</Button>
			</div>
		</form>
	</div>

	<!-- Tier Info -->
	<div class="mt-6 rounded-xl border border-gray-200 bg-gray-50 p-4">
		<h3 class="mb-2 text-sm font-medium text-gray-700">Service Tier Info</h3>
		{#if tier === 'light'}
			<p class="text-sm text-gray-600">
				<strong>Light:</strong> AI-powered transcription only. Fast turnaround, ideal for routine documentation.
			</p>
		{:else}
			<p class="text-sm text-gray-600">
				<strong>Pro:</strong> AI transcription with professional nurse review. Recommended for complex medical documents.
			</p>
		{/if}
	</div>
</div>

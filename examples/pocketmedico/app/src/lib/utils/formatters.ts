// Date formatters
export function formatDate(date: Date): string {
	return new Intl.DateTimeFormat('de-DE', {
		day: '2-digit',
		month: '2-digit',
		year: 'numeric'
	}).format(date);
}

export function formatDateTime(date: Date): string {
	return new Intl.DateTimeFormat('de-DE', {
		day: '2-digit',
		month: '2-digit',
		year: 'numeric',
		hour: '2-digit',
		minute: '2-digit'
	}).format(date);
}

export function formatRelativeTime(date: Date): string {
	const now = new Date();
	const diffMs = now.getTime() - date.getTime();
	const diffMins = Math.floor(diffMs / 60000);
	const diffHours = Math.floor(diffMs / 3600000);
	const diffDays = Math.floor(diffMs / 86400000);

	if (diffMins < 1) return 'Just now';
	if (diffMins < 60) return `${diffMins}m ago`;
	if (diffHours < 24) return `${diffHours}h ago`;
	if (diffDays < 7) return `${diffDays}d ago`;
	return formatDate(date);
}

// File size formatter
export function formatFileSize(bytes: number): string {
	if (bytes === 0) return '0 B';
	const k = 1024;
	const sizes = ['B', 'KB', 'MB', 'GB'];
	const i = Math.floor(Math.log(bytes) / Math.log(k));
	return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

// Duration formatter (seconds to mm:ss)
export function formatDuration(seconds: number): string {
	const mins = Math.floor(seconds / 60);
	const secs = seconds % 60;
	return `${mins}:${secs.toString().padStart(2, '0')}`;
}

// Order status display
export function formatOrderStatus(status: string): string {
	const statusMap: Record<string, string> = {
		pending: 'Pending',
		processing: 'Processing',
		ai_complete: 'AI Complete',
		in_review: 'In Review',
		ready: 'Ready',
		downloaded: 'Downloaded',
		error: 'Error'
	};
	return statusMap[status] || status;
}

// Document type display
export function formatDocumentType(type: string): string {
	const typeMap: Record<string, string> = {
		medical_report: 'Medical Report',
		discharge_summary: 'Discharge Summary',
		consultation_note: 'Consultation Note',
		prescription: 'Prescription',
		other: 'Other'
	};
	return typeMap[type] || type;
}

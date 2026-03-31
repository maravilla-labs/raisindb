// Auth
export { currentUser, isAuthenticated, isCustomer, isNurse, login, logout, restoreSession } from './auth';

// Users
export { users, findUserByEmail, registerUser, validateCredentials } from './users';
export type { User, UserRole, MockUser } from './users';

// Orders
export {
	orders,
	customerOrders,
	nurseQueue,
	createOrder,
	addFileToOrder,
	removeFileFromOrder,
	submitOrder,
	updateTranscript,
	approveTranscript,
	markAsDownloaded,
	getOrderById,
	generateOrderNumber
} from './orders';
export type { Order, OrderStatus, ServiceTier, DocumentType, UploadedFile, Transcript } from './orders';

// Inbox
export {
	inboxItems,
	userInbox,
	unreadCount,
	markAsRead,
	markAllAsRead,
	addInboxItem,
	deleteInboxItem
} from './inbox';
export type { InboxItem, InboxItemType } from './inbox';

// Toast
export { toasts } from './toast';
export type { Toast } from './toast';

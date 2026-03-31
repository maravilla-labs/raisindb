/**
 * Transaction support for RaisinDB
 */

import {
  RequestContext,
  RequestType,
  TransactionBeginPayload,
  TransactionBeginResponse,
  TransactionCommitPayload,
  TransactionCommitResponse,
  TransactionRollbackPayload,
} from './protocol';
import { NodeOperations } from './nodes';

/**
 * Transaction options
 */
export interface TransactionOptions {
  /** Optional commit message */
  message?: string;
  /** Optional actor identifier */
  actor?: string;
}

/**
 * Transaction class for executing multiple operations atomically
 */
export class Transaction {
  private sendRequest: (payload: unknown, requestType: string, contextOverride?: RequestContext) => Promise<unknown>;
  private context: RequestContext;
  private transactionId?: string;
  private isActive: boolean = false;
  private nodeOps?: NodeOperations;

  constructor(
    context: RequestContext,
    sendRequest: (payload: unknown, requestType: string, contextOverride?: RequestContext) => Promise<unknown>
  ) {
    this.context = context;
    this.sendRequest = sendRequest;
  }

  /**
   * Begin the transaction
   *
   * @param options - Transaction options
   * @returns This transaction instance for chaining
   */
  async begin(options?: TransactionOptions): Promise<Transaction> {
    if (this.isActive) {
      throw new Error('Transaction already active');
    }

    const payload: TransactionBeginPayload = {
      message: options?.message,
      actor: options?.actor,
    };

    const response = await this.sendRequest(
      payload,
      RequestType.TransactionBegin,
      this.context
    ) as TransactionBeginResponse;

    this.transactionId = response.transaction_id;
    this.isActive = true;

    // Create NodeOperations with transaction context
    const txContext: RequestContext = {
      ...this.context,
      transaction_id: this.transactionId,
    };

    this.nodeOps = new NodeOperations(
      (payload: unknown, requestType?: string) =>
        this.sendRequest(payload, requestType || 'node_create', txContext),
      txContext.workspace
    );

    return this;
  }

  /**
   * Get node operations scoped to this transaction
   *
   * @returns NodeOperations instance
   */
  nodes(): NodeOperations {
    if (!this.isActive || !this.nodeOps) {
      throw new Error('Transaction not active. Call begin() first.');
    }
    return this.nodeOps;
  }

  /**
   * Commit the transaction
   *
   * @returns Transaction commit response
   */
  async commit(): Promise<TransactionCommitResponse> {
    if (!this.isActive) {
      throw new Error('Transaction not active');
    }

    const payload: TransactionCommitPayload = {};

    const txContext: RequestContext = {
      ...this.context,
      transaction_id: this.transactionId,
    };

    try {
      const response = await this.sendRequest(
        payload,
        RequestType.TransactionCommit,
        txContext
      ) as TransactionCommitResponse;

      this.isActive = false;
      this.transactionId = undefined;
      this.nodeOps = undefined;

      return response;
    } catch (error) {
      // Transaction failed, mark as inactive
      this.isActive = false;
      this.transactionId = undefined;
      this.nodeOps = undefined;
      throw error;
    }
  }

  /**
   * Rollback the transaction
   */
  async rollback(): Promise<void> {
    if (!this.isActive) {
      throw new Error('Transaction not active');
    }

    const payload: TransactionRollbackPayload = {};

    const txContext: RequestContext = {
      ...this.context,
      transaction_id: this.transactionId,
    };

    try {
      await this.sendRequest(
        payload,
        RequestType.TransactionRollback,
        txContext
      );

      this.isActive = false;
      this.transactionId = undefined;
      this.nodeOps = undefined;
    } catch (error) {
      // Even if rollback fails, mark as inactive
      this.isActive = false;
      this.transactionId = undefined;
      this.nodeOps = undefined;
      throw error;
    }
  }

  /**
   * Get transaction ID
   */
  getTransactionId(): string | undefined {
    return this.transactionId;
  }

  /**
   * Check if transaction is active
   */
  isTransactionActive(): boolean {
    return this.isActive;
  }
}

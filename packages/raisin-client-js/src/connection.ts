/**
 * Universal WebSocket connection manager
 *
 * Detects environment (browser vs Node.js) and uses appropriate WebSocket implementation.
 * Handles auto-reconnect, heartbeat, and connection state management.
 */

import { EventEmitter } from 'events';
import { ReconnectManager, ReconnectOptions } from './utils/reconnect';
import { logger } from './logger';

/**
 * Connection state
 */
export enum ConnectionState {
  Disconnected = 'disconnected',
  Connecting = 'connecting',
  Connected = 'connected',
  Reconnecting = 'reconnecting',
  Closed = 'closed',
}

/**
 * Connection options
 */
export interface ConnectionOptions {
  /** Auto-reconnect on disconnect (default: true) */
  autoReconnect?: boolean;
  /** Reconnection options */
  reconnectOptions?: ReconnectOptions;
  /** Heartbeat interval in milliseconds (default: 30000, 0 to disable) */
  heartbeatInterval?: number;
  /** Heartbeat timeout in milliseconds (default: 5000) */
  heartbeatTimeout?: number;
  /** WebSocket protocols */
  protocols?: string | string[];
}

/**
 * Connection events
 */
export interface ConnectionEvents {
  stateChange: (state: ConnectionState) => void;
  message: (data: ArrayBuffer) => void;
  error: (error: Error) => void;
  close: (event: CloseEvent) => void;
}

/**
 * Type for WebSocket implementation (browser or Node.js)
 */
type WebSocketImpl = typeof WebSocket | typeof import('ws').WebSocket;

/**
 * Get the appropriate WebSocket implementation for the current environment
 */
function getWebSocketImpl(): WebSocketImpl {
  // Browser environment
  if (typeof WebSocket !== 'undefined') {
    return WebSocket as WebSocketImpl;
  }

  // Node.js environment
  try {
    // Use dynamic import to avoid bundler issues
    const ws = require('ws');
    return ws.WebSocket || ws.default || ws;
  } catch (error) {
    throw new Error(
      'WebSocket implementation not found. In Node.js, please install the "ws" package: npm install ws'
    );
  }
}

/**
 * Universal WebSocket connection manager
 */
export class Connection extends EventEmitter {
  private ws?: WebSocket;
  private url: string;
  private state: ConnectionState = ConnectionState.Disconnected;
  private reconnectManager: ReconnectManager;
  private heartbeatTimer?: NodeJS.Timeout;
  private heartbeatTimeoutTimer?: NodeJS.Timeout;
  private options: Required<Omit<ConnectionOptions, 'reconnectOptions' | 'protocols'>> & {
    protocols?: string | string[];
  };
  private WebSocketImpl: WebSocketImpl;
  private manualClose = false;

  constructor(url: string, options: ConnectionOptions = {}) {
    super();
    // Convert raisin:// / raisins:// to ws:// / wss:// for the WebSocket layer
    this.url = url
      .replace(/^raisin:\/\//, 'ws://')
      .replace(/^raisins:\/\//, 'wss://');
    this.options = {
      autoReconnect: options.autoReconnect ?? true,
      heartbeatInterval: options.heartbeatInterval ?? 30000,
      heartbeatTimeout: options.heartbeatTimeout ?? 5000,
      protocols: options.protocols,
    };
    this.reconnectManager = new ReconnectManager(options.reconnectOptions);
    this.WebSocketImpl = getWebSocketImpl();
  }

  /**
   * Connect to the WebSocket server
   */
  async connect(): Promise<void> {
    if (this.state === ConnectionState.Connected || this.state === ConnectionState.Connecting) {
      return;
    }

    this.manualClose = false;
    this.setState(ConnectionState.Connecting);

    return new Promise((resolve, reject) => {
      try {
        // Create WebSocket instance
        this.ws = new (this.WebSocketImpl as any)(
          this.url,
          this.options.protocols
        ) as WebSocket;

        // Set binary type for MessagePack
        this.ws.binaryType = 'arraybuffer';

        // Handle open event
        this.ws.onopen = () => {
          this.setState(ConnectionState.Connected);
          this.reconnectManager.reset();
          this.startHeartbeat();
          resolve();
        };

        // Handle message event
        this.ws.onmessage = (event: MessageEvent) => {
          logger.debug(`WebSocket onmessage fired - data type: ${typeof event.data}, is ArrayBuffer: ${event.data instanceof ArrayBuffer}`);
          this.resetHeartbeat();
          const data = event.data as ArrayBuffer;
          logger.debug(`Received binary message - size: ${data.byteLength} bytes`);
          logger.debug(`Emitting 'message' event to client handlers`);
          this.emit('message', data);
        };

        // Handle error event
        this.ws.onerror = () => {
          const error = new Error('WebSocket error occurred');
          this.emit('error', error);
          if (this.state === ConnectionState.Connecting) {
            reject(error);
          }
        };

        // Handle close event
        this.ws.onclose = (event: CloseEvent) => {
          this.stopHeartbeat();
          this.emit('close', event);

          if (!this.manualClose && this.options.autoReconnect) {
            this.setState(ConnectionState.Reconnecting);
            this.reconnect();
          } else {
            this.setState(ConnectionState.Disconnected);
          }

          if (this.state === ConnectionState.Connecting) {
            reject(new Error(`Connection closed: ${event.reason || 'Unknown reason'}`));
          }
        };
      } catch (error) {
        this.setState(ConnectionState.Disconnected);
        reject(error);
      }
    });
  }

  /**
   * Disconnect from the WebSocket server
   */
  disconnect(): void {
    this.manualClose = true;
    this.reconnectManager.cancelReconnect();
    this.stopHeartbeat();

    if (this.ws) {
      this.ws.close(1000, 'Client disconnect');
      this.ws = undefined;
    }

    this.setState(ConnectionState.Closed);
  }

  /**
   * Send data through the WebSocket
   *
   * @param data - Data to send (Uint8Array for MessagePack)
   */
  send(data: Uint8Array): void {
    if (this.state !== ConnectionState.Connected || !this.ws) {
      throw new Error('Cannot send data: not connected');
    }

    // Check actual WebSocket readyState
    if (this.ws.readyState !== 1) { // WebSocket.OPEN = 1
      logger.error(`WebSocket not open - readyState: ${this.ws.readyState} (0=CONNECTING, 1=OPEN, 2=CLOSING, 3=CLOSED)`);
      throw new Error(`Cannot send data: WebSocket not open (readyState: ${this.ws.readyState})`);
    }

    logger.debug(`Sending message - size: ${data.length} bytes, readyState: ${this.ws.readyState}`);

    try {
      this.ws.send(data);
      logger.debug(`Message sent successfully`);
    } catch (error) {
      logger.error(`Failed to send message:`, error);
      throw new Error(`Failed to send data: ${error}`);
    }
  }

  /**
   * Get current connection state
   */
  getState(): ConnectionState {
    return this.state;
  }

  /**
   * Check if connected
   */
  isConnected(): boolean {
    return this.state === ConnectionState.Connected;
  }

  /**
   * Reconnect to the server
   */
  private reconnect(): void {
    const success = this.reconnectManager.scheduleReconnect(async () => {
      try {
        await this.connect();
      } catch (error) {
        // Error is already handled by connect()
        // Reconnection will be scheduled again if autoReconnect is enabled
      }
    });

    if (!success) {
      this.setState(ConnectionState.Disconnected);
      this.emit('error', new Error('Max reconnection attempts reached'));
    }
  }

  /**
   * Set connection state and emit event
   */
  private setState(state: ConnectionState): void {
    if (this.state !== state) {
      this.state = state;
      this.emit('stateChange', state);
    }
  }

  /**
   * Start heartbeat mechanism
   */
  private startHeartbeat(): void {
    if (this.options.heartbeatInterval <= 0) {
      return;
    }

    this.stopHeartbeat();

    // Note: WebSocket implementations (browser and ws library) automatically handle
    // ping/pong frames at the protocol level, so we don't need application-level heartbeat
    // that sends empty binary messages. The underlying WebSocket will keep the connection alive.
  }

  /**
   * Stop heartbeat mechanism
   */
  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = undefined;
    }
    if (this.heartbeatTimeoutTimer) {
      clearTimeout(this.heartbeatTimeoutTimer);
      this.heartbeatTimeoutTimer = undefined;
    }
  }

  /**
   * Reset heartbeat timeout (called when message received)
   */
  private resetHeartbeat(): void {
    if (this.heartbeatTimeoutTimer) {
      clearTimeout(this.heartbeatTimeoutTimer);
      this.heartbeatTimeoutTimer = undefined;
    }
  }
}

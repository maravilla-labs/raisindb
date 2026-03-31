import * as THREE from 'three';
import { XRHandModelFactory } from 'three/examples/jsm/webxr/XRHandModelFactory.js';

// Extended XR Hand type that includes joints map
interface XRHandWithJoints extends THREE.Group {
  joints: Record<XRHandJoint, THREE.Object3D>;
}

export interface HandJoints {
  wrist: THREE.Object3D | null;
  thumbTip: THREE.Object3D | null;
  indexTip: THREE.Object3D | null;
  middleTip: THREE.Object3D | null;
  ringTip: THREE.Object3D | null;
  pinkyTip: THREE.Object3D | null;
}

export interface HandState {
  connected: boolean;
  joints: HandJoints;
  pinchStrength: number;  // 0-1, 1 = fully pinched
  isPinching: boolean;
  pinchPosition: THREE.Vector3 | null;
}

export type HandSide = 'left' | 'right';

export class HandTracker {
  private renderer: THREE.WebGLRenderer;
  private scene: THREE.Scene;
  private handModelFactory: XRHandModelFactory;

  private leftHand: XRHandWithJoints | null = null;
  private rightHand: XRHandWithJoints | null = null;
  private leftHandModel: THREE.Object3D | null = null;
  private rightHandModel: THREE.Object3D | null = null;

  private leftHandState: HandState = this.createEmptyHandState();
  private rightHandState: HandState = this.createEmptyHandState();

  // Pinch detection thresholds
  private readonly PINCH_START_THRESHOLD = 0.02;  // 2cm
  private readonly PINCH_END_THRESHOLD = 0.04;    // 4cm

  // Callbacks
  onHandConnected?: (side: HandSide) => void;
  onHandDisconnected?: (side: HandSide) => void;
  onPinchStart?: (side: HandSide, position: THREE.Vector3) => void;
  onPinchEnd?: (side: HandSide, position: THREE.Vector3) => void;
  onPinchMove?: (side: HandSide, position: THREE.Vector3) => void;

  constructor(renderer: THREE.WebGLRenderer, scene: THREE.Scene) {
    this.renderer = renderer;
    this.scene = scene;
    this.handModelFactory = new XRHandModelFactory();
  }

  private createEmptyHandState(): HandState {
    return {
      connected: false,
      joints: {
        wrist: null,
        thumbTip: null,
        indexTip: null,
        middleTip: null,
        ringTip: null,
        pinkyTip: null
      },
      pinchStrength: 0,
      isPinching: false,
      pinchPosition: null
    };
  }

  setup() {
    // Get XR hands from the renderer
    const xr = this.renderer.xr;

    // Setup left hand (index 0)
    this.leftHand = xr.getHand(0) as XRHandWithJoints;
    this.leftHandModel = this.handModelFactory.createHandModel(this.leftHand, 'mesh');
    this.leftHand.add(this.leftHandModel);
    this.scene.add(this.leftHand);

    // Setup right hand (index 1)
    this.rightHand = xr.getHand(1) as XRHandWithJoints;
    this.rightHandModel = this.handModelFactory.createHandModel(this.rightHand, 'mesh');
    this.rightHand.add(this.rightHandModel);
    this.scene.add(this.rightHand);

    // Listen for hand connection events
    this.leftHand.addEventListener('connected', () => {
      this.leftHandState.connected = true;
      this.onHandConnected?.('left');
    });

    this.leftHand.addEventListener('disconnected', () => {
      this.leftHandState = this.createEmptyHandState();
      this.onHandDisconnected?.('left');
    });

    this.rightHand.addEventListener('connected', () => {
      this.rightHandState.connected = true;
      this.onHandConnected?.('right');
    });

    this.rightHand.addEventListener('disconnected', () => {
      this.rightHandState = this.createEmptyHandState();
      this.onHandDisconnected?.('right');
    });
  }

  update(frame: XRFrame | undefined) {
    if (!frame) return;

    // Update left hand
    if (this.leftHand && this.leftHandState.connected) {
      this.updateHandState(this.leftHand, this.leftHandState, 'left');
    }

    // Update right hand
    if (this.rightHand && this.rightHandState.connected) {
      this.updateHandState(this.rightHand, this.rightHandState, 'right');
    }
  }

  private updateHandState(hand: XRHandWithJoints, state: HandState, side: HandSide) {
    // Get joint positions
    const joints = hand.joints;
    if (!joints) return;

    state.joints.wrist = joints['wrist'] || null;
    state.joints.thumbTip = joints['thumb-tip'] || null;
    state.joints.indexTip = joints['index-finger-tip'] || null;
    state.joints.middleTip = joints['middle-finger-tip'] || null;
    state.joints.ringTip = joints['ring-finger-tip'] || null;
    state.joints.pinkyTip = joints['pinky-finger-tip'] || null;

    // Calculate pinch (thumb tip to index tip distance)
    if (state.joints.thumbTip && state.joints.indexTip) {
      const thumbPos = new THREE.Vector3();
      const indexPos = new THREE.Vector3();
      state.joints.thumbTip.getWorldPosition(thumbPos);
      state.joints.indexTip.getWorldPosition(indexPos);

      const distance = thumbPos.distanceTo(indexPos);

      // Calculate pinch strength (inverted and normalized)
      const strength = Math.max(0, Math.min(1,
        1 - (distance - this.PINCH_START_THRESHOLD) /
        (this.PINCH_END_THRESHOLD - this.PINCH_START_THRESHOLD)
      ));
      state.pinchStrength = strength;

      // Calculate pinch midpoint
      const midpoint = new THREE.Vector3().lerpVectors(thumbPos, indexPos, 0.5);
      state.pinchPosition = midpoint;

      // Detect pinch state transitions
      const wasPinching = state.isPinching;

      if (distance < this.PINCH_START_THRESHOLD && !wasPinching) {
        state.isPinching = true;
        this.onPinchStart?.(side, midpoint);
      } else if (distance > this.PINCH_END_THRESHOLD && wasPinching) {
        state.isPinching = false;
        this.onPinchEnd?.(side, midpoint);
      } else if (state.isPinching) {
        this.onPinchMove?.(side, midpoint);
      }
    }
  }

  getLeftHandState(): HandState {
    return this.leftHandState;
  }

  getRightHandState(): HandState {
    return this.rightHandState;
  }

  getHandState(side: HandSide): HandState {
    return side === 'left' ? this.leftHandState : this.rightHandState;
  }

  isPinching(side?: HandSide): boolean {
    if (side === 'left') return this.leftHandState.isPinching;
    if (side === 'right') return this.rightHandState.isPinching;
    return this.leftHandState.isPinching || this.rightHandState.isPinching;
  }

  getPinchPosition(side: HandSide): THREE.Vector3 | null {
    const state = this.getHandState(side);
    return state.pinchPosition;
  }

  dispose() {
    if (this.leftHand) {
      this.scene.remove(this.leftHand);
    }
    if (this.rightHand) {
      this.scene.remove(this.rightHand);
    }
  }
}

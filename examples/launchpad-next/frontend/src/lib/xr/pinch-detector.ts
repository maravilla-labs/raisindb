import * as THREE from 'three';
import type { HandTracker, HandSide } from './hand-tracker';

export interface GrabbableObject {
  mesh: THREE.Object3D;
  id: string;
  originalPosition: THREE.Vector3;
  data: unknown;
}

export interface GrabState {
  isGrabbing: boolean;
  grabbedObject: GrabbableObject | null;
  grabHand: HandSide | null;
  grabOffset: THREE.Vector3;
}

export class PinchInteraction {
  private handTracker: HandTracker;
  private grabbableObjects: Map<string, GrabbableObject> = new Map();
  private grabState: GrabState = {
    isGrabbing: false,
    grabbedObject: null,
    grabHand: null,
    grabOffset: new THREE.Vector3()
  };

  private readonly GRAB_DISTANCE = 0.1; // 10cm grab radius

  // Callbacks
  onGrab?: (object: GrabbableObject, hand: HandSide) => void;
  onRelease?: (object: GrabbableObject, hand: HandSide, position: THREE.Vector3) => void;
  onMove?: (object: GrabbableObject, position: THREE.Vector3) => void;

  constructor(handTracker: HandTracker) {
    this.handTracker = handTracker;

    // Listen for pinch events
    this.handTracker.onPinchStart = (side, position) => {
      this.handlePinchStart(side, position);
    };

    this.handTracker.onPinchEnd = (side, position) => {
      this.handlePinchEnd(side, position);
    };

    this.handTracker.onPinchMove = (side, position) => {
      this.handlePinchMove(side, position);
    };
  }

  registerGrabbable(mesh: THREE.Object3D, id: string, data?: unknown) {
    const grabbable: GrabbableObject = {
      mesh,
      id,
      originalPosition: mesh.position.clone(),
      data
    };
    this.grabbableObjects.set(id, grabbable);
  }

  unregisterGrabbable(id: string) {
    this.grabbableObjects.delete(id);
  }

  clearGrabbables() {
    this.grabbableObjects.clear();
  }

  private handlePinchStart(side: HandSide, position: THREE.Vector3) {
    // Don't grab if already grabbing
    if (this.grabState.isGrabbing) return;

    // Find closest grabbable object within grab distance
    let closestObject: GrabbableObject | null = null;
    let closestDistance = Infinity;

    for (const [, grabbable] of this.grabbableObjects) {
      const worldPos = new THREE.Vector3();
      grabbable.mesh.getWorldPosition(worldPos);
      const distance = position.distanceTo(worldPos);

      if (distance < this.GRAB_DISTANCE && distance < closestDistance) {
        closestDistance = distance;
        closestObject = grabbable;
      }
    }

    if (closestObject) {
      const worldPos = new THREE.Vector3();
      closestObject.mesh.getWorldPosition(worldPos);

      this.grabState = {
        isGrabbing: true,
        grabbedObject: closestObject,
        grabHand: side,
        grabOffset: worldPos.clone().sub(position)
      };

      this.onGrab?.(closestObject, side);
    }
  }

  private handlePinchEnd(side: HandSide, position: THREE.Vector3) {
    // Only release if this hand is holding something
    if (!this.grabState.isGrabbing || this.grabState.grabHand !== side) return;

    const releasedObject = this.grabState.grabbedObject;

    if (releasedObject) {
      this.onRelease?.(releasedObject, side, position);
    }

    this.grabState = {
      isGrabbing: false,
      grabbedObject: null,
      grabHand: null,
      grabOffset: new THREE.Vector3()
    };
  }

  private handlePinchMove(side: HandSide, position: THREE.Vector3) {
    // Only move if this hand is holding something
    if (!this.grabState.isGrabbing || this.grabState.grabHand !== side) return;

    const grabbedObject = this.grabState.grabbedObject;
    if (!grabbedObject) return;

    // Calculate new position with offset
    const newPosition = position.clone().add(this.grabState.grabOffset);

    // Update mesh position (in world space)
    // Need to convert to local space if mesh has a parent
    const parent = grabbedObject.mesh.parent;
    if (parent) {
      parent.worldToLocal(newPosition);
    }

    grabbedObject.mesh.position.copy(newPosition);

    this.onMove?.(grabbedObject, newPosition);
  }

  isGrabbing(): boolean {
    return this.grabState.isGrabbing;
  }

  getGrabbedObject(): GrabbableObject | null {
    return this.grabState.grabbedObject;
  }

  getGrabHand(): HandSide | null {
    return this.grabState.grabHand;
  }

  // Reset grabbed object to its original position
  resetGrabbedObject() {
    if (this.grabState.grabbedObject) {
      this.grabState.grabbedObject.mesh.position.copy(
        this.grabState.grabbedObject.originalPosition
      );
    }
  }

  // Update original position (call after successful drop)
  updateOriginalPosition(id: string, newPosition: THREE.Vector3) {
    const grabbable = this.grabbableObjects.get(id);
    if (grabbable) {
      grabbable.originalPosition.copy(newPosition);
    }
  }
}

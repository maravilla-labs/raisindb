import * as THREE from 'three';
import type { HandTracker, HandSide, HandState } from './hand-tracker';

export interface GrabbableObject {
  mesh: THREE.Object3D;
  id: string;
  originalPosition: THREE.Vector3;
  data: unknown;
}

export interface RayPointerConfig {
  rayLength?: number;
  rayColor?: number;
  rayActiveColor?: number;
  grabDistance?: number;
}

const DEFAULT_CONFIG: Required<RayPointerConfig> = {
  rayLength: 3.0,        // 3 meters
  rayColor: 0x8b5cf6,    // Purple
  rayActiveColor: 0x22c55e, // Green when pointing at something
  grabDistance: 5.0      // Can grab up to 5m away
};

export class RayPointerInteraction {
  private handTracker: HandTracker;
  private scene: THREE.Scene;
  private config: Required<RayPointerConfig>;
  private grabbableObjects: Map<string, GrabbableObject> = new Map();

  // Ray visuals
  private leftRay: THREE.Line | null = null;
  private rightRay: THREE.Line | null = null;
  private leftRayMaterial: THREE.LineBasicMaterial | null = null;
  private rightRayMaterial: THREE.LineBasicMaterial | null = null;

  // Raycaster for hit testing
  private raycaster = new THREE.Raycaster();

  // Grab state
  private grabbedObject: GrabbableObject | null = null;
  private grabHand: HandSide | null = null;
  private grabDistance: number = 0; // Distance from hand when grabbed

  // Hover state
  private hoveredObject: GrabbableObject | null = null;
  private hoverHand: HandSide | null = null;

  // Callbacks
  onGrab?: (object: GrabbableObject, hand: HandSide) => void;
  onRelease?: (object: GrabbableObject, hand: HandSide, position: THREE.Vector3) => void;
  onMove?: (object: GrabbableObject, position: THREE.Vector3) => void;
  onHover?: (object: GrabbableObject | null, hand: HandSide | null) => void;

  constructor(handTracker: HandTracker, scene: THREE.Scene, config: RayPointerConfig = {}) {
    this.handTracker = handTracker;
    this.scene = scene;
    this.config = { ...DEFAULT_CONFIG, ...config };

    this.createRays();
    this.setupPinchHandlers();
  }

  private createRays() {
    // Create ray geometry (line from origin to forward)
    const points = [
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(0, 0, -this.config.rayLength)
    ];
    const geometry = new THREE.BufferGeometry().setFromPoints(points);

    // Left ray
    this.leftRayMaterial = new THREE.LineBasicMaterial({
      color: this.config.rayColor,
      transparent: true,
      opacity: 0.6
    });
    this.leftRay = new THREE.Line(geometry.clone(), this.leftRayMaterial);
    this.leftRay.visible = false;
    this.scene.add(this.leftRay);

    // Right ray
    this.rightRayMaterial = new THREE.LineBasicMaterial({
      color: this.config.rayColor,
      transparent: true,
      opacity: 0.6
    });
    this.rightRay = new THREE.Line(geometry.clone(), this.rightRayMaterial);
    this.rightRay.visible = false;
    this.scene.add(this.rightRay);
  }

  private setupPinchHandlers() {
    this.handTracker.onPinchStart = (side, position) => {
      this.handlePinchStart(side);
    };

    this.handTracker.onPinchEnd = (side, position) => {
      this.handlePinchEnd(side);
    };
  }

  private handlePinchStart(side: HandSide) {
    // If already grabbing, ignore
    if (this.grabbedObject) return;

    // Check if we're pointing at something
    const hit = this.raycast(side);
    if (hit) {
      this.grabbedObject = hit.object;
      this.grabHand = side;
      this.grabDistance = hit.distance;
      this.onGrab?.(hit.object, side);
    }
  }

  private handlePinchEnd(side: HandSide) {
    // Only release if this hand is holding something
    if (!this.grabbedObject || this.grabHand !== side) return;

    const worldPos = new THREE.Vector3();
    this.grabbedObject.mesh.getWorldPosition(worldPos);

    this.onRelease?.(this.grabbedObject, side, worldPos);

    this.grabbedObject = null;
    this.grabHand = null;
    this.grabDistance = 0;
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

  // Call this every frame to update rays and interactions
  update() {
    this.updateRay('left');
    this.updateRay('right');
    this.updateGrabbedObject();
    this.updateHover();
  }

  private updateRay(side: HandSide) {
    const ray = side === 'left' ? this.leftRay : this.rightRay;
    const material = side === 'left' ? this.leftRayMaterial : this.rightRayMaterial;
    if (!ray || !material) return;

    const handState = this.handTracker.getHandState(side);

    if (!handState.connected || !handState.joints.indexTip || !handState.joints.wrist) {
      ray.visible = false;
      return;
    }

    // Get positions
    const indexPos = new THREE.Vector3();
    const wristPos = new THREE.Vector3();
    handState.joints.indexTip.getWorldPosition(indexPos);
    handState.joints.wrist.getWorldPosition(wristPos);

    // Ray direction: from wrist toward finger tip (forward from hand)
    const direction = indexPos.clone().sub(wristPos).normalize();

    // Position ray at index finger tip
    ray.position.copy(indexPos);

    // Set rotation to point along direction
    // Create a quaternion that rotates from default -Z to our direction
    const quaternion = new THREE.Quaternion();
    quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, -1), direction);
    ray.quaternion.copy(quaternion);

    ray.visible = true;

    // Update color based on hover/grab state
    if (this.grabHand === side && this.grabbedObject) {
      material.color.setHex(this.config.rayActiveColor);
      material.opacity = 0.9;
    } else if (this.hoverHand === side && this.hoveredObject) {
      material.color.setHex(this.config.rayActiveColor);
      material.opacity = 0.7;
    } else {
      material.color.setHex(this.config.rayColor);
      material.opacity = 0.4;
    }
  }

  private updateGrabbedObject() {
    if (!this.grabbedObject || !this.grabHand) return;

    const handState = this.handTracker.getHandState(this.grabHand);
    if (!handState.connected || !handState.joints.indexTip || !handState.joints.wrist) return;

    // Calculate ray from hand
    const indexPos = new THREE.Vector3();
    const wristPos = new THREE.Vector3();
    handState.joints.indexTip.getWorldPosition(indexPos);
    handState.joints.wrist.getWorldPosition(wristPos);

    const direction = indexPos.clone().sub(wristPos).normalize();

    // Position object along the ray at the grab distance
    const newWorldPos = indexPos.clone().add(direction.multiplyScalar(this.grabDistance));

    // Convert to local space if mesh has a parent
    const parent = this.grabbedObject.mesh.parent;
    if (parent) {
      parent.worldToLocal(newWorldPos);
    }

    this.grabbedObject.mesh.position.copy(newWorldPos);
    this.onMove?.(this.grabbedObject, newWorldPos);
  }

  private updateHover() {
    // Don't update hover while grabbing
    if (this.grabbedObject) {
      if (this.hoveredObject) {
        this.onHover?.(null, null);
        this.hoveredObject = null;
        this.hoverHand = null;
      }
      return;
    }

    // Check both hands for hover
    let newHovered: GrabbableObject | null = null;
    let newHoverHand: HandSide | null = null;
    let closestDist = Infinity;

    for (const side of ['left', 'right'] as HandSide[]) {
      const hit = this.raycast(side);
      if (hit && hit.distance < closestDist) {
        newHovered = hit.object;
        newHoverHand = side;
        closestDist = hit.distance;
      }
    }

    if (newHovered !== this.hoveredObject) {
      this.hoveredObject = newHovered;
      this.hoverHand = newHoverHand;
      this.onHover?.(newHovered, newHoverHand);
    }
  }

  private raycast(side: HandSide): { object: GrabbableObject; distance: number } | null {
    const handState = this.handTracker.getHandState(side);
    if (!handState.connected || !handState.joints.indexTip || !handState.joints.wrist) {
      return null;
    }

    // Get ray from hand
    const indexPos = new THREE.Vector3();
    const wristPos = new THREE.Vector3();
    handState.joints.indexTip.getWorldPosition(indexPos);
    handState.joints.wrist.getWorldPosition(wristPos);

    const direction = indexPos.clone().sub(wristPos).normalize();

    this.raycaster.set(indexPos, direction);
    this.raycaster.far = this.config.grabDistance;

    // Collect all meshes to test
    const meshes: THREE.Object3D[] = [];
    for (const grabbable of this.grabbableObjects.values()) {
      meshes.push(grabbable.mesh);
    }

    const intersects = this.raycaster.intersectObjects(meshes, true);

    if (intersects.length > 0) {
      const hit = intersects[0];
      // Find which grabbable this belongs to
      for (const grabbable of this.grabbableObjects.values()) {
        if (hit.object === grabbable.mesh || grabbable.mesh.getObjectById(hit.object.id)) {
          return { object: grabbable, distance: hit.distance };
        }
      }
    }

    return null;
  }

  isGrabbing(): boolean {
    return this.grabbedObject !== null;
  }

  getGrabbedObject(): GrabbableObject | null {
    return this.grabbedObject;
  }

  getGrabHand(): HandSide | null {
    return this.grabHand;
  }

  resetGrabbedObject() {
    if (this.grabbedObject) {
      this.grabbedObject.mesh.position.copy(this.grabbedObject.originalPosition);
    }
  }

  updateOriginalPosition(id: string, newPosition: THREE.Vector3) {
    const grabbable = this.grabbableObjects.get(id);
    if (grabbable) {
      grabbable.originalPosition.copy(newPosition);
    }
  }

  dispose() {
    if (this.leftRay) {
      this.scene.remove(this.leftRay);
      this.leftRay.geometry.dispose();
      this.leftRayMaterial?.dispose();
    }
    if (this.rightRay) {
      this.scene.remove(this.rightRay);
      this.rightRay.geometry.dispose();
      this.rightRayMaterial?.dispose();
    }
  }
}

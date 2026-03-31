import * as THREE from 'three';
import type { HandTracker, HandSide } from './hand-tracker';

export interface InteractableObject {
  mesh: THREE.Object3D;
  id: string;
  data?: unknown;
}

/**
 * Simplified hand interaction system:
 * - Index finger pointing for hover/selection
 * - Pinch gesture for zooming the board
 * - Touch cards directly to select them
 */
export class HandInteraction {
  private handTracker: HandTracker;
  private scene: THREE.Scene;
  private boardGroup: THREE.Group;

  // Interactable objects (cards)
  private interactables: Map<string, InteractableObject> = new Map();

  // Visual feedback
  private pointerDot: THREE.Mesh | null = null;
  private pointerRay: THREE.Line | null = null;

  // Raycaster
  private raycaster = new THREE.Raycaster();

  // State
  private hoveredObject: InteractableObject | null = null;
  private selectedObject: InteractableObject | null = null;
  private isZooming = false;
  private initialPinchDistance = 0;
  private initialBoardScale = 1;

  // Zoom limits
  private readonly MIN_SCALE = 0.3;
  private readonly MAX_SCALE = 2.0;

  // Callbacks
  onHover?: (object: InteractableObject | null) => void;
  onSelect?: (object: InteractableObject) => void;
  onDeselect?: (object: InteractableObject) => void;

  constructor(handTracker: HandTracker, scene: THREE.Scene, boardGroup: THREE.Group) {
    this.handTracker = handTracker;
    this.scene = scene;
    this.boardGroup = boardGroup;

    this.createPointerVisuals();
  }

  private createPointerVisuals() {
    // Pointer dot (shows where finger is pointing)
    const dotGeometry = new THREE.SphereGeometry(0.008, 16, 16);
    const dotMaterial = new THREE.MeshBasicMaterial({
      color: 0x8b5cf6,  // Purple
      transparent: true,
      opacity: 0.8
    });
    this.pointerDot = new THREE.Mesh(dotGeometry, dotMaterial);
    this.pointerDot.visible = false;
    this.scene.add(this.pointerDot);

    // Pointer ray (subtle line from finger)
    const rayGeometry = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(0, 0, 0),
      new THREE.Vector3(0, 0, -0.5)
    ]);
    const rayMaterial = new THREE.LineBasicMaterial({
      color: 0x8b5cf6,
      transparent: true,
      opacity: 0.3
    });
    this.pointerRay = new THREE.Line(rayGeometry, rayMaterial);
    this.pointerRay.visible = false;
    this.scene.add(this.pointerRay);
  }

  registerInteractable(mesh: THREE.Object3D, id: string, data?: unknown) {
    this.interactables.set(id, { mesh, id, data });
  }

  unregisterInteractable(id: string) {
    this.interactables.delete(id);
  }

  clearInteractables() {
    this.interactables.clear();
  }

  update() {
    const leftState = this.handTracker.getHandState('left');
    const rightState = this.handTracker.getHandState('right');

    // Check for two-hand pinch zoom
    if (leftState.connected && rightState.connected &&
        leftState.isPinching && rightState.isPinching) {
      this.handleTwoHandZoom(leftState, rightState);
      this.hidePointer();
      return;
    } else {
      this.isZooming = false;
    }

    // Use dominant hand (right) for pointing, fallback to left
    const activeHand = rightState.connected ? rightState :
                       leftState.connected ? leftState : null;
    const activeSide: HandSide = rightState.connected ? 'right' : 'left';

    if (!activeHand || !activeHand.joints.indexTip) {
      this.hidePointer();
      return;
    }

    // Get index finger position and direction
    const indexTip = activeHand.joints.indexTip;
    const wrist = activeHand.joints.wrist;

    if (!indexTip || !wrist) {
      this.hidePointer();
      return;
    }

    const tipPos = new THREE.Vector3();
    const wristPos = new THREE.Vector3();
    indexTip.getWorldPosition(tipPos);
    wrist.getWorldPosition(wristPos);

    // Direction from wrist through finger tip (pointing direction)
    const direction = tipPos.clone().sub(wristPos).normalize();

    // Update pointer ray visual
    this.updatePointerRay(tipPos, direction);

    // Raycast to find what we're pointing at
    this.raycaster.set(tipPos, direction);
    this.raycaster.far = 3.0;  // 3 meter range

    const meshes = Array.from(this.interactables.values()).map(i => i.mesh);
    const intersects = this.raycaster.intersectObjects(meshes, true);

    if (intersects.length > 0) {
      const hit = intersects[0];

      // Find which interactable was hit
      const hitObject = this.findInteractableByMesh(hit.object);

      if (hitObject && hitObject !== this.hoveredObject) {
        this.hoveredObject = hitObject;
        this.onHover?.(hitObject);
      }

      // Show pointer dot at hit point
      this.showPointerAt(hit.point, true);

      // Check for pinch to select
      if (activeHand.isPinching && hitObject && !this.selectedObject) {
        this.selectedObject = hitObject;
        this.onSelect?.(hitObject);
      }
    } else {
      if (this.hoveredObject) {
        this.hoveredObject = null;
        this.onHover?.(null);
      }
      // Show pointer dot at end of ray
      const endPoint = tipPos.clone().add(direction.multiplyScalar(0.5));
      this.showPointerAt(endPoint, false);
    }

    // Check for pinch release to deselect
    if (this.selectedObject && !activeHand.isPinching) {
      this.onDeselect?.(this.selectedObject);
      this.selectedObject = null;
    }
  }

  private handleTwoHandZoom(leftState: any, rightState: any) {
    const leftPos = new THREE.Vector3();
    const rightPos = new THREE.Vector3();

    if (leftState.joints.indexTip && rightState.joints.indexTip) {
      leftState.joints.indexTip.getWorldPosition(leftPos);
      rightState.joints.indexTip.getWorldPosition(rightPos);

      const currentDistance = leftPos.distanceTo(rightPos);

      if (!this.isZooming) {
        // Start zooming
        this.isZooming = true;
        this.initialPinchDistance = currentDistance;
        this.initialBoardScale = this.boardGroup.scale.x;
      } else {
        // Continue zooming - scale based on pinch distance change
        const scaleFactor = currentDistance / this.initialPinchDistance;
        const newScale = Math.max(this.MIN_SCALE,
                         Math.min(this.MAX_SCALE,
                         this.initialBoardScale * scaleFactor));

        this.boardGroup.scale.setScalar(newScale);
      }
    }
  }

  private findInteractableByMesh(mesh: THREE.Object3D): InteractableObject | null {
    for (const interactable of this.interactables.values()) {
      if (mesh === interactable.mesh || interactable.mesh.getObjectById(mesh.id)) {
        return interactable;
      }
    }
    return null;
  }

  private updatePointerRay(origin: THREE.Vector3, direction: THREE.Vector3) {
    if (!this.pointerRay) return;

    this.pointerRay.position.copy(origin);

    // Point ray in direction
    const quaternion = new THREE.Quaternion();
    quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, -1), direction);
    this.pointerRay.quaternion.copy(quaternion);

    this.pointerRay.visible = true;
  }

  private showPointerAt(position: THREE.Vector3, isHovering: boolean) {
    if (!this.pointerDot) return;

    this.pointerDot.position.copy(position);
    this.pointerDot.visible = true;

    // Change color based on hover state
    const material = this.pointerDot.material as THREE.MeshBasicMaterial;
    if (isHovering) {
      material.color.setHex(0x22c55e);  // Green when hovering
      material.opacity = 1.0;
      this.pointerDot.scale.setScalar(1.5);
    } else {
      material.color.setHex(0x8b5cf6);  // Purple normally
      material.opacity = 0.6;
      this.pointerDot.scale.setScalar(1.0);
    }
  }

  private hidePointer() {
    if (this.pointerDot) this.pointerDot.visible = false;
    if (this.pointerRay) this.pointerRay.visible = false;
  }

  getHoveredObject(): InteractableObject | null {
    return this.hoveredObject;
  }

  getSelectedObject(): InteractableObject | null {
    return this.selectedObject;
  }

  isCurrentlyZooming(): boolean {
    return this.isZooming;
  }

  getBoardScale(): number {
    return this.boardGroup.scale.x;
  }

  dispose() {
    if (this.pointerDot) {
      this.scene.remove(this.pointerDot);
      this.pointerDot.geometry.dispose();
      (this.pointerDot.material as THREE.Material).dispose();
    }
    if (this.pointerRay) {
      this.scene.remove(this.pointerRay);
      this.pointerRay.geometry.dispose();
      (this.pointerRay.material as THREE.Material).dispose();
    }
  }
}

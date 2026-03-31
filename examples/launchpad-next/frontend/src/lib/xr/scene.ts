import * as THREE from 'three';

export interface SceneConfig {
  boardPosition?: THREE.Vector3;
  columnWidth?: number;
  columnHeight?: number;
  columnGap?: number;
  cardWidth?: number;
  cardHeight?: number;
  cardDepth?: number;
  cardGap?: number;
}

const DEFAULT_CONFIG: Required<SceneConfig> = {
  boardPosition: new THREE.Vector3(0, 1.2, -1.0), // 1m in front, 1.2m high (chest level)
  columnWidth: 0.3,    // 30cm
  columnHeight: 0.5,   // 50cm
  columnGap: 0.35,     // 35cm spacing
  cardWidth: 0.25,     // 25cm
  cardHeight: 0.08,    // 8cm
  cardDepth: 0.01,     // 1cm
  cardGap: 0.02        // 2cm between cards
};

export class XRScene {
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  renderer: THREE.WebGLRenderer;
  config: Required<SceneConfig>;

  private boardGroup: THREE.Group;
  private animationId: number | null = null;

  constructor(container: HTMLElement, config: SceneConfig = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };

    // Create scene
    this.scene = new THREE.Scene();

    // Create camera (will be controlled by XR)
    this.camera = new THREE.PerspectiveCamera(
      70,
      container.clientWidth / container.clientHeight,
      0.01,
      20
    );

    // Create renderer with transparency for passthrough
    this.renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: true
    });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.setSize(container.clientWidth, container.clientHeight);
    this.renderer.xr.enabled = true;
    this.renderer.sortObjects = true;

    container.appendChild(this.renderer.domElement);

    // Setup lighting
    this.setupLighting();

    // Create board group
    this.boardGroup = new THREE.Group();
    this.boardGroup.position.copy(this.config.boardPosition);
    this.scene.add(this.boardGroup);

    // Handle resize
    window.addEventListener('resize', () => this.handleResize(container));
  }

  private setupLighting() {
    // Ambient light for overall illumination - brighter for AR
    const ambientLight = new THREE.AmbientLight(0xffffff, 1.0);
    this.scene.add(ambientLight);

    // Main directional light from above-front
    const directionalLight = new THREE.DirectionalLight(0xffffff, 0.6);
    directionalLight.position.set(0, 2, 2);
    this.scene.add(directionalLight);

    // Fill light from front to ensure glass is visible
    const frontLight = new THREE.DirectionalLight(0xffffff, 0.4);
    frontLight.position.set(0, 0, 3);
    this.scene.add(frontLight);

    // Subtle rim light for depth
    const rimLight = new THREE.DirectionalLight(0xffffff, 0.2);
    rimLight.position.set(0, 0, -2);
    this.scene.add(rimLight);
  }

  private handleResize(container: HTMLElement) {
    this.camera.aspect = container.clientWidth / container.clientHeight;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(container.clientWidth, container.clientHeight);
  }

  getBoardGroup(): THREE.Group {
    return this.boardGroup;
  }

  // Move the board to a new position
  setBoardPosition(position: THREE.Vector3) {
    this.boardGroup.position.copy(position);
  }

  // Get current board position
  getBoardPosition(): THREE.Vector3 {
    return this.boardGroup.position.clone();
  }

  getRenderer(): THREE.WebGLRenderer {
    return this.renderer;
  }

  getScene(): THREE.Scene {
    return this.scene;
  }

  getCamera(): THREE.PerspectiveCamera {
    return this.camera;
  }

  startRenderLoop(onFrame?: (time: number, frame?: XRFrame) => void) {
    this.renderer.setAnimationLoop((time, frame) => {
      if (onFrame) {
        onFrame(time, frame);
      }
      this.renderer.render(this.scene, this.camera);
    });
  }

  stopRenderLoop() {
    this.renderer.setAnimationLoop(null);
  }

  dispose() {
    this.stopRenderLoop();
    this.renderer.dispose();

    // Dispose all objects in the scene
    this.scene.traverse((object) => {
      if (object instanceof THREE.Mesh) {
        object.geometry.dispose();
        if (Array.isArray(object.material)) {
          object.material.forEach(m => m.dispose());
        } else {
          object.material.dispose();
        }
      }
    });
  }
}

export function createXRScene(container: HTMLElement, config?: SceneConfig): XRScene {
  return new XRScene(container, config);
}

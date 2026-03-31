import * as THREE from 'three';

export interface Card3DData {
  uuid: string;
  title: string;
  description?: string;
  columnId: string;
  index: number;
}

export interface Card3DConfig {
  width?: number;
  height?: number;
  depth?: number;
  cornerRadius?: number;
}

const DEFAULT_CONFIG: Required<Card3DConfig> = {
  width: 0.26,     // 26cm
  height: 0.055,   // 5.5cm
  depth: 0.005,    // 5mm - thinner
  cornerRadius: 0.006  // 6mm radius for rounded corners
};

// Helper to create a rounded rectangle shape
function createRoundedRectShape(width: number, height: number, radius: number): THREE.Shape {
  const shape = new THREE.Shape();
  const x = -width / 2;
  const y = -height / 2;

  shape.moveTo(x + radius, y);
  shape.lineTo(x + width - radius, y);
  shape.quadraticCurveTo(x + width, y, x + width, y + radius);
  shape.lineTo(x + width, y + height - radius);
  shape.quadraticCurveTo(x + width, y + height, x + width - radius, y + height);
  shape.lineTo(x + radius, y + height);
  shape.quadraticCurveTo(x, y + height, x, y + height - radius);
  shape.lineTo(x, y + radius);
  shape.quadraticCurveTo(x, y, x + radius, y);

  return shape;
}

export class Card3D {
  mesh: THREE.Group;
  data: Card3DData;
  config: Required<Card3DConfig>;

  private bodyMesh: THREE.Mesh;
  private accentMesh: THREE.Mesh;  // Purple bar on left
  private textMesh: THREE.Mesh;
  private backTextMesh: THREE.Mesh;
  private textTexture: THREE.CanvasTexture | null = null;
  private bodyMaterial: THREE.MeshStandardMaterial;
  private isHovered: boolean = false;
  private isGrabbed: boolean = false;

  // Colors matching web version
  private static readonly CARD_COLOR = 0xffffff;
  private static readonly CARD_HOVER_COLOR = 0xfaf5ff;
  private static readonly CARD_GRABBED_COLOR = 0xf3e8ff;
  private static readonly ACCENT_COLOR = 0x8b5cf6;  // Purple
  private static readonly ACCENT_GRABBED = 0x7c3aed;

  constructor(data: Card3DData, config: Card3DConfig = {}) {
    this.data = data;
    this.config = { ...DEFAULT_CONFIG, ...config };

    this.mesh = new THREE.Group();
    this.mesh.name = `card-${this.data.uuid}`;
    this.mesh.userData.cardId = this.data.uuid;
    this.mesh.userData.card3d = this;

    // Create rounded rectangle shape for the card body
    const cardShape = createRoundedRectShape(
      this.config.width,
      this.config.height,
      this.config.cornerRadius
    );

    // Extrude the shape to create 3D geometry
    const extrudeSettings: THREE.ExtrudeGeometryOptions = {
      depth: this.config.depth,
      bevelEnabled: false  // No bevel for cleaner look
    };

    const bodyGeometry = new THREE.ExtrudeGeometry(cardShape, extrudeSettings);
    // Center the geometry on Z axis
    bodyGeometry.translate(0, 0, -this.config.depth / 2);

    // Create card body with frosted glass effect (no transmission for XR compatibility)
    this.bodyMaterial = new THREE.MeshStandardMaterial({
      color: 0xffffff,
      roughness: 0.7,
      metalness: 0.05,
      transparent: true,
      opacity: 0.5,
      side: THREE.DoubleSide,
      depthWrite: false
    });

    this.bodyMesh = new THREE.Mesh(bodyGeometry, this.bodyMaterial);
    this.bodyMesh.renderOrder = 10;
    this.mesh.add(this.bodyMesh);

    // Purple accent bar on left edge (solid, not transparent)
    const accentWidth = 0.005;  // 5mm
    const accentHeight = this.config.height * 0.65;
    const accentShape = createRoundedRectShape(accentWidth, accentHeight, 0.0025);
    const accentExtrudeSettings: THREE.ExtrudeGeometryOptions = {
      depth: this.config.depth + 0.001,
      bevelEnabled: false
    };
    const accentGeometry = new THREE.ExtrudeGeometry(accentShape, accentExtrudeSettings);
    accentGeometry.translate(0, 0, -(this.config.depth + 0.001) / 2);

    const accentMaterial = new THREE.MeshStandardMaterial({
      color: Card3D.ACCENT_COLOR,
      roughness: 0.4,
      metalness: 0.1,
      transparent: false,
      emissive: Card3D.ACCENT_COLOR,
      emissiveIntensity: 0.2
    });
    this.accentMesh = new THREE.Mesh(accentGeometry, accentMaterial);
    this.accentMesh.position.x = -this.config.width / 2 + accentWidth / 2 + this.config.cornerRadius + 0.003;
    this.accentMesh.renderOrder = 11;
    this.mesh.add(this.accentMesh);

    // Create text planes for front and back
    const textGeometry = new THREE.PlaneGeometry(
      this.config.width * 0.78,
      this.config.height * 0.7
    );

    const textMaterial = new THREE.MeshBasicMaterial({
      transparent: true,
      depthWrite: false,
      depthTest: true
    });

    this.textMesh = new THREE.Mesh(textGeometry, textMaterial);
    this.textMesh.position.x = 0.015;  // Offset right of accent bar
    this.textMesh.position.z = this.config.depth / 2 + 0.002;
    this.textMesh.renderOrder = 12;
    this.mesh.add(this.textMesh);

    // Back text
    const backTextMaterial = new THREE.MeshBasicMaterial({
      transparent: true,
      depthWrite: false
    });
    this.backTextMesh = new THREE.Mesh(textGeometry.clone(), backTextMaterial);
    this.backTextMesh.position.x = 0.015;
    this.backTextMesh.position.z = -(this.config.depth / 2 + 0.002);
    this.backTextMesh.rotation.y = Math.PI;
    this.backTextMesh.renderOrder = 12;
    this.mesh.add(this.backTextMesh);

    this.updateTextTexture();
  }

  private updateTextTexture() {
    // Use power-of-2 dimensions for better WebGL compatibility
    const canvas = document.createElement('canvas');
    canvas.width = 512;
    canvas.height = 64;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Clear with transparent background
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // Draw title
    ctx.fillStyle = '#1e293b';  // Dark gray
    ctx.font = 'bold 20px Arial, sans-serif';
    ctx.textBaseline = 'middle';

    // Truncate title if too long
    const maxWidth = 480;
    let title = this.data.title;
    let metrics = ctx.measureText(title);
    while (metrics.width > maxWidth && title.length > 3) {
      title = title.slice(0, -4) + '...';
      metrics = ctx.measureText(title);
    }

    const yPos = this.data.description ? 20 : 32;
    ctx.fillText(title, 8, yPos);

    // Draw description if present
    if (this.data.description) {
      ctx.fillStyle = '#64748b';  // Lighter gray
      ctx.font = '14px Arial, sans-serif';

      let desc = this.data.description;
      metrics = ctx.measureText(desc);
      while (metrics.width > maxWidth && desc.length > 3) {
        desc = desc.slice(0, -4) + '...';
        metrics = ctx.measureText(desc);
      }

      ctx.fillText(desc, 8, 46);
    }

    // Create texture
    if (this.textTexture) {
      this.textTexture.dispose();
    }

    this.textTexture = new THREE.CanvasTexture(canvas);
    this.textTexture.needsUpdate = true;
    this.textTexture.minFilter = THREE.LinearFilter;
    this.textTexture.magFilter = THREE.LinearFilter;

    // Apply to front
    const frontMaterial = this.textMesh.material as THREE.MeshBasicMaterial;
    frontMaterial.map = this.textTexture;
    frontMaterial.needsUpdate = true;

    // Create mirrored texture for back
    const backCanvas = document.createElement('canvas');
    backCanvas.width = 512;
    backCanvas.height = 64;
    const backCtx = backCanvas.getContext('2d');
    if (backCtx) {
      backCtx.translate(canvas.width, 0);
      backCtx.scale(-1, 1);
      backCtx.drawImage(canvas, 0, 0);

      const backTexture = new THREE.CanvasTexture(backCanvas);
      backTexture.needsUpdate = true;
      backTexture.minFilter = THREE.LinearFilter;

      const backMaterial = this.backTextMesh.material as THREE.MeshBasicMaterial;
      backMaterial.map = backTexture;
      backMaterial.needsUpdate = true;
    }
  }

  setHovered(hovered: boolean) {
    if (this.isHovered === hovered) return;
    this.isHovered = hovered;
    this.updateVisualState();
  }

  setGrabbed(grabbed: boolean) {
    if (this.isGrabbed === grabbed) return;
    this.isGrabbed = grabbed;
    this.updateVisualState();
  }

  private updateVisualState() {
    const accentMaterial = this.accentMesh.material as THREE.MeshStandardMaterial;

    if (this.isGrabbed) {
      // More opaque and purple tinted when grabbed
      this.bodyMaterial.opacity = 0.75;
      this.bodyMaterial.color.setHex(0xf3e8ff);  // Light purple
      this.bodyMaterial.emissive.setHex(Card3D.ACCENT_COLOR);
      this.bodyMaterial.emissiveIntensity = 0.2;
      accentMaterial.color.setHex(Card3D.ACCENT_GRABBED);
      accentMaterial.emissive.setHex(Card3D.ACCENT_COLOR);
      accentMaterial.emissiveIntensity = 0.5;
      this.mesh.scale.setScalar(1.08);
    } else if (this.isHovered) {
      // Slightly more visible when hovered
      this.bodyMaterial.opacity = 0.6;
      this.bodyMaterial.color.setHex(0xfaf5ff);  // Very light purple
      this.bodyMaterial.emissive.setHex(Card3D.ACCENT_COLOR);
      this.bodyMaterial.emissiveIntensity = 0.1;
      accentMaterial.color.setHex(Card3D.ACCENT_COLOR);
      accentMaterial.emissive.setHex(Card3D.ACCENT_COLOR);
      accentMaterial.emissiveIntensity = 0.3;
      this.mesh.scale.setScalar(1.03);
    } else {
      // Default transparent state
      this.bodyMaterial.opacity = 0.5;
      this.bodyMaterial.color.setHex(0xffffff);
      this.bodyMaterial.emissive.setHex(0x000000);
      this.bodyMaterial.emissiveIntensity = 0;
      accentMaterial.color.setHex(Card3D.ACCENT_COLOR);
      accentMaterial.emissive.setHex(Card3D.ACCENT_COLOR);
      accentMaterial.emissiveIntensity = 0.2;
      this.mesh.scale.setScalar(1.0);
    }
  }

  updateData(data: Partial<Card3DData>) {
    Object.assign(this.data, data);
    this.updateTextTexture();
  }

  getPosition(): THREE.Vector3 {
    return this.mesh.position.clone();
  }

  setPosition(x: number, y: number, z: number) {
    this.mesh.position.set(x, y, z);
  }

  dispose() {
    if (this.textTexture) {
      this.textTexture.dispose();
    }

    this.bodyMaterial.dispose();
    this.bodyMesh.geometry.dispose();

    (this.accentMesh.material as THREE.Material).dispose();
    this.accentMesh.geometry.dispose();

    (this.textMesh.material as THREE.Material).dispose();
    this.textMesh.geometry.dispose();

    (this.backTextMesh.material as THREE.Material).dispose();
    this.backTextMesh.geometry.dispose();
  }
}

export function createCard3D(data: Card3DData, config?: Card3DConfig): Card3D {
  return new Card3D(data, config);
}

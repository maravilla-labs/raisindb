import * as THREE from 'three';
import { Card3D, type Card3DData, type Card3DConfig } from './card-3d';

export interface Column3DData {
  id: string;
  title: string;
  cards: Array<{
    uuid: string;
    title: string;
    description?: string;
  }>;
}

export interface Column3DConfig {
  width?: number;
  height?: number;
  depth?: number;
  headerHeight?: number;
  cardGap?: number;
  padding?: number;
  cornerRadius?: number;
}

const DEFAULT_CONFIG: Required<Column3DConfig> = {
  width: 0.3,       // 30cm
  height: 0.5,      // 50cm (will be calculated dynamically)
  depth: 0.015,     // 1.5cm - thinner for better look
  headerHeight: 0.045, // 4.5cm
  cardGap: 0.012,   // 1.2cm between cards
  padding: 0.015,   // 1.5cm padding
  cornerRadius: 0.012 // 1.2cm radius for rounded corners
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

export class Column3D {
  group: THREE.Group;
  data: Column3DData;
  config: Required<Column3DConfig>;
  cardConfig: Required<Card3DConfig>;

  private headerMesh: THREE.Mesh | null = null;
  private backgroundMesh: THREE.Mesh | null = null;
  private cards: Map<string, Card3D> = new Map();
  private dropZone: THREE.Mesh | null = null;
  private isDropTarget: boolean = false;

  // Colors matching web version with glass effect
  private static readonly COLUMN_BG_COLOR = 0xe2e8f0;      // Light gray
  private static readonly COLUMN_HEADER_BG = 0x94a3b8;     // Gray header background
  private static readonly COLUMN_HEADER_TEXT = 0x475569;   // Dark gray text
  private static readonly DROP_ZONE_COLOR = 0x8b5cf6;      // Purple

  constructor(
    data: Column3DData,
    config: Column3DConfig = {},
    cardConfig: Card3DConfig = {}
  ) {
    this.data = data;
    this.cardConfig = {
      width: 0.26,    // Slightly smaller than column
      height: 0.055,  // 5.5cm height
      depth: 0.006,   // Thinner
      cornerRadius: 0.006,  // 6mm radius for rounded corners
      ...cardConfig
    };

    // Calculate dynamic height based on number of cards
    const baseConfig = { ...DEFAULT_CONFIG, ...config };
    const minHeight = 0.15; // Minimum column height
    const cardCount = Math.max(data.cards.length, 1);
    const calculatedHeight =
      baseConfig.padding * 2 +           // Top and bottom padding
      baseConfig.headerHeight +           // Header
      baseConfig.padding +                // Gap after header
      (cardCount * this.cardConfig.height) +  // Cards
      ((cardCount - 1) * baseConfig.cardGap) + // Gaps between cards
      baseConfig.padding;                 // Extra bottom padding

    this.config = {
      ...baseConfig,
      height: Math.max(minHeight, calculatedHeight)
    };

    this.group = new THREE.Group();
    this.group.name = `column-${data.id}`;

    this.createBackground();
    this.createHeader();
    this.createDropZone();
    this.createCards();
  }

  private createBackground() {
    // Create rounded rectangle shape for the column
    const columnShape = createRoundedRectShape(
      this.config.width,
      this.config.height,
      this.config.cornerRadius
    );

    // Extrude to create 3D geometry with rounded corners
    const extrudeSettings: THREE.ExtrudeGeometryOptions = {
      depth: this.config.depth,
      bevelEnabled: true,
      bevelThickness: 0.001,
      bevelSize: 0.001,
      bevelSegments: 2
    };

    const geometry = new THREE.ExtrudeGeometry(columnShape, extrudeSettings);
    // Center on Z axis and push back
    geometry.translate(0, 0, -this.config.depth);

    // Frosted glass effect - simple transparency (no transmission for XR compatibility)
    const material = new THREE.MeshStandardMaterial({
      color: 0xe8ecf0,          // Light grayish
      roughness: 0.9,
      metalness: 0.05,
      transparent: true,
      opacity: 0.35,
      side: THREE.DoubleSide,
      depthWrite: false
    });

    this.backgroundMesh = new THREE.Mesh(geometry, material);
    this.backgroundMesh.renderOrder = 0;
    this.group.add(this.backgroundMesh);
  }

  private createHeader() {
    // Header background - rounded pill shape
    const headerWidth = this.config.width - this.config.padding * 2;
    const headerRadius = this.config.headerHeight / 2; // Full pill shape

    const headerShape = createRoundedRectShape(headerWidth, this.config.headerHeight, headerRadius);
    const extrudeSettings: THREE.ExtrudeGeometryOptions = {
      depth: 0.004,
      bevelEnabled: false
    };

    const geometry = new THREE.ExtrudeGeometry(headerShape, extrudeSettings);
    geometry.translate(0, 0, -0.002);

    // Slightly more opaque frosted glass for header
    const material = new THREE.MeshStandardMaterial({
      color: 0xd0d8e0,          // Slightly darker for header
      roughness: 0.8,
      metalness: 0.05,
      transparent: true,
      opacity: 0.5,
      side: THREE.DoubleSide,
      depthWrite: false
    });

    this.headerMesh = new THREE.Mesh(geometry, material);
    this.headerMesh.position.y = this.config.height / 2 - this.config.headerHeight / 2 - this.config.padding;
    this.headerMesh.position.z = 0.002;
    this.headerMesh.renderOrder = 1;
    this.group.add(this.headerMesh);

    // Add title text and card count
    this.addHeaderText();
  }

  private addHeaderText() {
    // Create canvas for header text
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    // Use power-of-2 dimensions for better WebGL compatibility
    canvas.width = 512;
    canvas.height = 64;

    // Clear with transparent background
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // Draw column title (uppercase, bold)
    ctx.fillStyle = '#1e293b';  // Dark text for contrast
    ctx.font = 'bold 24px Arial, sans-serif';
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    ctx.fillText(this.data.title.toUpperCase(), 16, 32);

    // Draw card count on right side
    const countText = String(this.data.cards.length);
    ctx.font = 'bold 20px Arial, sans-serif';
    ctx.textAlign = 'right';
    ctx.fillStyle = '#64748b';
    ctx.fillText(countText, canvas.width - 16, 32);

    const texture = new THREE.CanvasTexture(canvas);
    texture.needsUpdate = true;
    texture.minFilter = THREE.LinearFilter;
    texture.magFilter = THREE.LinearFilter;

    const textGeometry = new THREE.PlaneGeometry(
      this.config.width - this.config.padding * 2,
      this.config.headerHeight * 0.8
    );

    const textMaterial = new THREE.MeshBasicMaterial({
      map: texture,
      transparent: true,
      depthWrite: false,
      depthTest: true
    });

    // Front text
    const textMesh = new THREE.Mesh(textGeometry, textMaterial);
    textMesh.position.y = this.config.height / 2 - this.config.headerHeight / 2 - this.config.padding;
    textMesh.position.z = 0.008;
    textMesh.renderOrder = 2;
    this.group.add(textMesh);

    // Back text (mirrored)
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

      const backMaterial = new THREE.MeshBasicMaterial({
        map: backTexture,
        transparent: true,
        depthWrite: false
      });

      const backTextMesh = new THREE.Mesh(textGeometry.clone(), backMaterial);
      backTextMesh.position.y = textMesh.position.y;
      backTextMesh.position.z = -0.008;
      backTextMesh.rotation.y = Math.PI;
      backTextMesh.renderOrder = 2;
      this.group.add(backTextMesh);
    }
  }

  private createDropZone() {
    const dropZoneHeight = 0.015;
    const dropZoneWidth = this.config.width - this.config.padding * 2;
    const dropZoneRadius = dropZoneHeight / 2; // Pill shape

    const dropZoneShape = createRoundedRectShape(dropZoneWidth, dropZoneHeight, dropZoneRadius);
    const extrudeSettings: THREE.ExtrudeGeometryOptions = {
      depth: 0.004,
      bevelEnabled: false
    };

    const geometry = new THREE.ExtrudeGeometry(dropZoneShape, extrudeSettings);
    geometry.translate(0, 0, -0.002);

    const material = new THREE.MeshBasicMaterial({
      color: Column3D.DROP_ZONE_COLOR,
      transparent: true,
      opacity: 0
    });

    this.dropZone = new THREE.Mesh(geometry, material);
    this.dropZone.position.z = 0.007;
    this.dropZone.visible = false;
    this.group.add(this.dropZone);
  }

  private createCards() {
    const startY = this.config.height / 2 - this.config.headerHeight - this.config.padding * 2;

    this.data.cards.forEach((cardData, index) => {
      const card3DData: Card3DData = {
        uuid: cardData.uuid,
        title: cardData.title,
        description: cardData.description,
        columnId: this.data.id,
        index
      };

      const card = new Card3D(card3DData, this.cardConfig);

      // Position card within column - stagger z slightly to prevent z-fighting
      const y = startY - (this.cardConfig.height / 2) - (index * (this.cardConfig.height + this.config.cardGap));
      const z = 0.012 + (index * 0.001);  // Slight z offset per card
      card.setPosition(0, y, z);

      this.cards.set(cardData.uuid, card);
      this.group.add(card.mesh);
    });
  }

  getCard(uuid: string): Card3D | undefined {
    return this.cards.get(uuid);
  }

  getAllCards(): Card3D[] {
    return Array.from(this.cards.values());
  }

  setDropTarget(isTarget: boolean, dropIndex?: number) {
    if (this.isDropTarget === isTarget && !isTarget) return;

    this.isDropTarget = isTarget;

    if (this.dropZone) {
      const material = this.dropZone.material as THREE.MeshBasicMaterial;
      material.opacity = isTarget ? 0.7 : 0;
      this.dropZone.visible = isTarget;

      if (isTarget && dropIndex !== undefined) {
        const startY = this.config.height / 2 - this.config.headerHeight - this.config.padding * 2.5;
        const y = startY - (dropIndex * (this.cardConfig.height + this.config.cardGap));
        this.dropZone.position.y = y;
      }
    }
  }

  getDropIndex(worldPosition: THREE.Vector3): number {
    const localPos = this.group.worldToLocal(worldPosition.clone());
    const startY = this.config.height / 2 - this.config.headerHeight - this.config.padding * 2.5;

    const relativeY = startY - localPos.y;
    const cardSlotHeight = this.cardConfig.height + this.config.cardGap;

    let index = Math.round(relativeY / cardSlotHeight);
    index = Math.max(0, Math.min(index, this.data.cards.length));

    return index;
  }

  containsPoint(worldPosition: THREE.Vector3): boolean {
    const localPos = this.group.worldToLocal(worldPosition.clone());

    const halfWidth = this.config.width / 2;
    const halfHeight = this.config.height / 2;

    return (
      Math.abs(localPos.x) < halfWidth &&
      Math.abs(localPos.y) < halfHeight &&
      Math.abs(localPos.z) < 0.1
    );
  }

  repositionCards() {
    const startY = this.config.height / 2 - this.config.headerHeight - this.config.padding * 2;

    let index = 0;
    for (const card of this.cards.values()) {
      const y = startY - (this.cardConfig.height / 2) - (index * (this.cardConfig.height + this.config.cardGap));
      const z = 0.012 + (index * 0.001);
      card.setPosition(0, y, z);
      card.data.index = index;
      index++;
    }
  }

  dispose() {
    for (const card of this.cards.values()) {
      card.dispose();
    }
    this.cards.clear();

    this.group.traverse((object) => {
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

export function createColumn3D(
  data: Column3DData,
  config?: Column3DConfig,
  cardConfig?: Card3DConfig
): Column3D {
  return new Column3D(data, config, cardConfig);
}

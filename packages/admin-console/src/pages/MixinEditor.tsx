import NodeTypeEditor from './NodeTypeEditor'

// Mixins reuse the NodeType editor in "mixin" mode: same property builder and
// YAML editor, but inheritance panels (extends / mixins / allowed children) and
// the resolved view are hidden, and all reads/writes hit the /mixins API.
export default function MixinEditor() {
  return <NodeTypeEditor kind="mixin" />
}

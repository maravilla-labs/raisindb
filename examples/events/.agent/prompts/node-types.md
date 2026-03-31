# Prompt: Create a Node Type

Use this prompt to guide the creation of a new NodeType. Answer these questions
first, then generate the YAML schema.

## Questions to Answer

1. **What is the name?**
   Use the package namespace prefix: `{namespace}:{Name}`
   Example: `blog:Article`, `shop:Product`, `cms:Page`

2. **What does it represent?**
   Describe the content type in one sentence.

3. **What properties does it need?**
   List each field with:
   - Name (camelCase)
   - Type (String, Int, Float, Boolean, DateTime, Reference, Section, Json, StringList, ReferenceList)
   - Is it required?
   - Should it be indexed for queries?

4. **Does it need page templates (Archetypes)?**
   If content should be rendered as a page, create an Archetype that links to
   this NodeType, defines editor fields, and maps to a frontend page component.

5. **Which workspace(s) should allow it?**
   Add it to the workspace's `allowed_node_types` list.

6. **Does it relate to other types?**
   If it references other nodes, use Reference/ReferenceList properties and note
   the target NodeType.

## Output

Generate:
- `nodetypes/{namespace}:{Name}.yaml` -- the NodeType schema
- Update `manifest.yaml` -- add to `provides.nodetypes`
- Update workspace YAML -- add to `allowed_node_types` if applicable
- Optionally: `archetypes/{namespace}:{Name}.yaml` for page template + frontend component mapping

## Validation

After generating files, run:
```bash
raisindb package create --check .
```

//! Tests for validation logic

use super::context::ValidationContext;
use super::field_resolution::validate_element_content;
use super::helpers::{suggest_node_type_name, to_pascal_case};
use super::context::NODE_TYPE_NAME_REGEX;
use crate::errors::{FileType, ValidationResult};
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::element::field_types::FieldSchemaBase;
use raisin_models::nodes::types::Archetype;
use raisin_validation::field_helpers::is_required as get_field_required;
use super::{validate_content};

#[test]
fn test_flat_element_required_field_validation() {
    // Test that flat element format correctly validates required fields
    let element_type_yaml = r##"
name: test:Hero
title: Hero Section
description: Full-width hero section
icon: image
color: "#8b5cf6"
version: 1
fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true
  - $type: TextField
    name: required_field
    title: Required Field
    required: true
"##;

    let element_type: ElementType = serde_yaml::from_str(element_type_yaml).unwrap();
    assert_eq!(element_type.name, "test:Hero");
    assert_eq!(element_type.fields.len(), 2);

    let mut ctx = ValidationContext::default();
    ctx.package_element_types.insert("test:Hero".to_string(), element_type);

    let content_yaml = r#"
element_type: test:Hero
uuid: test-123
headline: Test Headline
"#;
    let content: serde_yaml::Mapping = serde_yaml::from_str::<serde_yaml::Value>(content_yaml)
        .unwrap()
        .as_mapping()
        .unwrap()
        .clone();

    let mut result = ValidationResult::success(FileType::Content);
    validate_element_content(&content, "test:Hero", &ctx, "test.yaml", &mut result);

    assert_eq!(result.errors.len(), 1, "Expected 1 error for missing required_field, got: {:?}", result.errors);
    assert!(result.errors[0].message.contains("required_field"));
}

#[test]
fn test_full_package_validation_flow() {
    let element_type_yaml = r##"
name: launchpad:Hero
title: Hero Section
description: Full-width hero section
icon: image
color: "#8b5cf6"
version: 1
fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true
  - $type: TextField
    name: test_required_field
    title: Test Required Field
    required: true
"##;
    let element_type: ElementType = serde_yaml::from_str(element_type_yaml)
        .expect("Failed to parse element type");
    assert_eq!(element_type.name, "launchpad:Hero");

    let archetype_yaml = r##"
name: launchpad:LandingPage
title: Landing Page
description: Landing page template
icon: layout
color: "#6366f1"
base_node_type: launchpad:Page
version: 1
fields:
  - $type: TextField
    name: title
    title: Page Title
    required: true
  - $type: SectionField
    name: content
    title: Page Content
    allowed_element_types:
      - launchpad:Hero
"##;
    let archetype: Archetype = serde_yaml::from_str(archetype_yaml)
        .expect("Failed to parse archetype");
    assert_eq!(archetype.name, "launchpad:LandingPage");

    let mut ctx = ValidationContext::default();
    ctx.package_node_types.insert("launchpad:Page".to_string());
    ctx.package_archetypes.insert("launchpad:LandingPage".to_string(), archetype);
    ctx.package_element_types.insert("launchpad:Hero".to_string(), element_type);

    let content_yaml = r##"
node_type: launchpad:Page
archetype: launchpad:LandingPage
properties:
  title: Home
  content:
    - uuid: hero-1
      element_type: launchpad:Hero
      headline: Welcome
"##;

    let result = validate_content(content_yaml, "home/.node.yaml", &ctx);

    let has_required_field_error = result.errors.iter()
        .any(|e| e.message.contains("test_required_field"));

    assert!(has_required_field_error,
        "Expected error for missing test_required_field in Hero element. Errors: {:?}", result.errors);
}

#[test]
fn test_home_page_validation() {
    let archetype_yaml = r##"
name: launchpad:LandingPage
title: Landing Page
description: Landing page template with hero, content blocks, and features
icon: layout
color: "#6366f1"
base_node_type: launchpad:Page
version: 1

fields:
  - $type: TextField
    name: title
    title: Page Title
    required: true

  - $type: TextField
    name: slug
    title: URL Slug
    required: true

  - $type: TextField
    name: description
    title: Meta Description
    required: false

  - $type: SectionField
    name: content
    title: Page Content
    allowed_element_types:
      - launchpad:Hero
      - launchpad:TextBlock
      - launchpad:FeatureGrid
      - launchpad:ListKanbanBoards

publishable: true
"##;
    let archetype: Archetype = serde_yaml::from_str(archetype_yaml)
        .expect("Failed to parse archetype");

    assert!(archetype.fields.is_some());
    let fields = archetype.fields.as_ref().unwrap();
    let content_field = fields.iter().find(|f| f.base_name() == "content");
    assert!(content_field.is_some(), "Archetype should have 'content' SectionField");

    let hero_yaml = r##"
name: launchpad:Hero
title: Hero Section
description: Full-width hero section with headline, subheadline, and call-to-action
icon: image
color: "#8b5cf6"
version: 1

fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true

  - $type: TextField
    name: test_required_field
    title: Test Required Field
    required: true
"##;
    let element_type: ElementType = serde_yaml::from_str(hero_yaml)
        .expect("Failed to parse element type");

    let mut ctx = ValidationContext::default();
    ctx.package_node_types.insert("launchpad:Page".to_string());
    ctx.package_archetypes.insert("launchpad:LandingPage".to_string(), archetype);
    ctx.package_element_types.insert("launchpad:Hero".to_string(), element_type);

    let home_yaml = r##"
node_type: launchpad:Page
archetype: launchpad:LandingPage
properties:
  title: Welcome to Launchpad
  slug: home
  description: Your gateway to launching amazing projects
  content:
    - uuid: hero-1
      element_type: launchpad:Hero
      headline: Launch Your Vision
      subheadline: Build, deploy, and scale your ideas with Launchpad
      cta_text: Get Started
      cta_link: /contact
"##;

    let result = validate_content(home_yaml, "home/.node.yaml", &ctx);

    let has_error = result.errors.iter()
        .any(|e| e.message.contains("test_required_field"));
    assert!(has_error,
        "Should have error for missing test_required_field. Errors: {:?}", result.errors);
}

#[test]
fn test_parse_actual_hero_yaml() {
    let hero_yaml = r##"
name: launchpad:Hero
title: Hero Section
description: Full-width hero section with headline, subheadline, and call-to-action
icon: image
color: "#8b5cf6"
version: 1

fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true

  - $type: TextField
    name: subheadline
    title: Subheadline
    required: false

  - $type: TextField
    name: cta_text
    title: CTA Button Text
    required: false

  - $type: TextField
    name: cta_link
    title: CTA Button Link
    description: URL to navigate to when clicked
    required: false

  - $type: TextField
    name: cta_action
    title: CTA Action
    description: Action to trigger (e.g., createBoard). Used instead of cta_link.
    required: false

  - $type: MediaField
    name: background_image
    title: Background Image
    required: false

  # TEMPORARY: Testing validation - this required field is missing in content
  - $type: TextField
    name: test_required_field
    title: Test Required Field
    required: true
"##;

    let result = serde_yaml::from_str::<ElementType>(hero_yaml);
    let element_type = result.expect("Should parse hero.yaml");
    assert_eq!(element_type.name, "launchpad:Hero");
    assert_eq!(element_type.fields.len(), 7);

    let test_field = element_type.fields.iter()
        .find(|f| f.base_name() == "test_required_field");
    assert!(test_field.is_some(), "test_required_field should exist");
    assert!(get_field_required(test_field.unwrap()), "test_required_field should be required");
}

#[test]
fn test_node_type_name_regex() {
    assert!(NODE_TYPE_NAME_REGEX.is_match("raisin:Folder"));
    assert!(NODE_TYPE_NAME_REGEX.is_match("custom:MyType"));
    assert!(NODE_TYPE_NAME_REGEX.is_match("app:Article"));
    assert!(!NODE_TYPE_NAME_REGEX.is_match("folder"));
    assert!(!NODE_TYPE_NAME_REGEX.is_match("Folder"));
    assert!(!NODE_TYPE_NAME_REGEX.is_match("raisin:folder"));
}

#[test]
fn test_suggest_node_type_name() {
    assert_eq!(suggest_node_type_name("folder"), "custom:Folder");
    assert_eq!(suggest_node_type_name("my_type"), "custom:MyType");
    assert_eq!(suggest_node_type_name("raisin:folder"), "raisin:Folder");
}

#[test]
fn test_to_pascal_case() {
    assert_eq!(to_pascal_case("hello"), "Hello");
    assert_eq!(to_pascal_case("hello_world"), "HelloWorld");
    assert_eq!(to_pascal_case("hello-world"), "HelloWorld");
    assert_eq!(to_pascal_case("HELLO"), "Hello");
}

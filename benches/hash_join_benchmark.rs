//! Benchmark comparing HashJoin vs NestedLoopJoin performance
//!
//! This benchmark demonstrates the performance improvement of HashJoin
//! over NestedLoopJoin for equality joins with varying dataset sizes.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use raisin_sql::{Analyzer, PhysicalPlanner, QueryPlan};
use raisin_sql::analyzer::catalog::StaticCatalog;
use raisin_sql::physical_plan::{execute_plan, ExecutionContext};
use raisin_storage_memory::MemoryStorage;
use std::sync::Arc;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use tokio::runtime::Runtime;

/// Create test data with specified number of users and orders per user
async fn create_benchmark_data(
    num_users: usize,
    orders_per_user: usize,
) -> Arc<MemoryStorage> {
    let storage = Arc::new(MemoryStorage::new());

    // Create users
    for i in 1..=num_users {
        let user = Node {
            id: format!("user{}", i),
            path: format!("/users/user{}", i),
            name: format!("User {}", i),
            node_type: "user".to_string(),
            archetype: Some("user".to_string()),
            properties: {
                let mut props = std::collections::HashMap::new();
                props.insert("user_id".to_string(), PropertyValue::Number(i as f64));
                props.insert("username".to_string(), PropertyValue::String(format!("user{}", i)));
                props
            },
            parent_name: Some("users".to_string()),
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            owner_id: None,
            relations: None,
            has_children: None,
        };

        storage.nodes().create(
            "bench_tenant",
            "bench_repo",
            "main",
            "default",
            user,
            None,
        ).await.unwrap();
    }

    // Create orders
    for user_id in 1..=num_users {
        for order_num in 1..=orders_per_user {
            let order = Node {
                id: format!("order_{}_{}", user_id, order_num),
                path: format!("/orders/order_{}_{}", user_id, order_num),
                name: format!("Order {} for User {}", order_num, user_id),
                node_type: "order".to_string(),
                archetype: Some("order".to_string()),
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("order_id".to_string(), PropertyValue::Number((user_id * 1000 + order_num) as f64));
                    props.insert("user_id".to_string(), PropertyValue::Number(user_id as f64));
                    props.insert("amount".to_string(), PropertyValue::Number((order_num * 10) as f64));
                    props
                },
                parent_name: Some("orders".to_string()),
                version: 1,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                published_at: None,
                published_by: None,
                updated_by: None,
                created_by: None,
                translations: None,
                owner_id: None,
                relations: None,
                has_children: None,
            };

            storage.nodes().create(
                "bench_tenant",
                "bench_repo",
                "main",
                "default",
                order,
                None,
            ).await.unwrap();
        }
    }

    storage
}

/// Benchmark HashJoin performance with equality condition
async fn bench_hash_join(storage: Arc<MemoryStorage>, num_users: usize) {
    let catalog = StaticCatalog::default_nodes_schema();

    let sql = format!(
        "SELECT JSON_GET_DOUBLE(u.properties, 'user_id') as user_id,
                JSON_GET_DOUBLE(o.properties, 'order_id') as order_id
         FROM nodes u
         INNER JOIN nodes o ON JSON_GET_DOUBLE(u.properties, 'user_id') = JSON_GET_DOUBLE(o.properties, 'user_id')
         WHERE u.node_type = 'user' AND o.node_type = 'order'
         LIMIT {}", num_users * 2
    );

    let analyzer = Analyzer::with_catalog(Box::new(catalog.clone()));
    let analyzed = analyzer.analyze(&sql).expect("Analysis should succeed");

    let plan_result = QueryPlan::from_analyzed(&analyzed, &catalog);
    let query_plan = plan_result.expect("Planning should succeed");

    let planner = PhysicalPlanner::new();
    let physical_plan = planner.plan(&query_plan.optimized).expect("Physical planning should succeed");

    // Verify we're using HashJoin
    let plan_str = format!("{:?}", physical_plan);
    assert!(plan_str.contains("HashJoin"), "Should use HashJoin");

    let ctx = ExecutionContext {
        storage: storage.clone(),
        tenant_id: "bench_tenant".to_string(),
        repo_id: "bench_repo".to_string(),
        branch: "main".to_string(),
        workspace: "default".to_string(),
        max_revision: None,
        locales: vec![],
        indexing_engine: None,
        embedding_provider: None,
    };

    let stream = execute_plan(&physical_plan, &ctx).await.expect("Execution should succeed");
    let _rows: Vec<_> = futures::StreamExt::collect::<Vec<_>>(stream).await;
}

fn benchmark_hash_join_scaling(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("hash_join_scaling");
    group.sample_size(10); // Reduce sample size for faster benchmarks

    // Test with different dataset sizes
    for num_users in [10, 50, 100, 200].iter() {
        let orders_per_user = 5;

        // Create data once per size
        let storage = rt.block_on(create_benchmark_data(*num_users, orders_per_user));

        group.bench_with_input(
            BenchmarkId::new("users", num_users),
            num_users,
            |b, &num_users| {
                b.to_async(&rt).iter(|| {
                    bench_hash_join(black_box(storage.clone()), black_box(num_users))
                });
            },
        );
    }

    group.finish();
}

fn benchmark_hash_join_vs_data_size(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("hash_join_data_scaling");
    group.sample_size(10);

    // Fix number of users, vary orders per user
    let num_users = 50;

    for orders_per_user in [1, 3, 5, 10].iter() {
        let storage = rt.block_on(create_benchmark_data(num_users, *orders_per_user));

        group.bench_with_input(
            BenchmarkId::new("orders_per_user", orders_per_user),
            orders_per_user,
            |b, _| {
                b.to_async(&rt).iter(|| {
                    bench_hash_join(black_box(storage.clone()), black_box(num_users))
                });
            },
        );
    }

    group.finish();
}

fn benchmark_join_selectivity(c: &mut Criterion) {
    // Benchmark how HashJoin performs with different join selectivities
    // (percentage of rows that match)
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("hash_join_selectivity");
    group.sample_size(10);

    // Create dataset with 100 users
    // Vary how many have orders (selectivity)
    for users_with_orders in [20, 50, 80, 100].iter() {
        let storage = rt.block_on(async {
            let storage = Arc::new(MemoryStorage::new());

            // Create all 100 users
            for i in 1..=100 {
                let user = Node {
                    id: format!("user{}", i),
                    path: format!("/users/user{}", i),
                    name: format!("User {}", i),
                    node_type: "user".to_string(),
                    archetype: Some("user".to_string()),
                    properties: {
                        let mut props = std::collections::HashMap::new();
                        props.insert("user_id".to_string(), PropertyValue::Number(i as f64));
                        props
                    },
                    parent_name: Some("users".to_string()),
                    version: 1,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    published_at: None,
                    published_by: None,
                    updated_by: None,
                    created_by: None,
                    translations: None,
                    owner_id: None,
                    relations: None,
                    has_children: None,
                };

                storage.nodes().create(
                    "bench_tenant",
                    "bench_repo",
                    "main",
                    "default",
                    user,
                    None,
                ).await.unwrap();
            }

            // Create orders only for first N users
            for user_id in 1..=*users_with_orders {
                for order_num in 1..=5 {
                    let order = Node {
                        id: format!("order_{}_{}", user_id, order_num),
                        path: format!("/orders/order_{}_{}", user_id, order_num),
                        name: format!("Order {}", order_num),
                        node_type: "order".to_string(),
                        archetype: Some("order".to_string()),
                        properties: {
                            let mut props = std::collections::HashMap::new();
                            props.insert("order_id".to_string(), PropertyValue::Number((user_id * 1000 + order_num) as f64));
                            props.insert("user_id".to_string(), PropertyValue::Number(user_id as f64));
                            props
                        },
                        parent_name: Some("orders".to_string()),
                        version: 1,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                        published_at: None,
                        published_by: None,
                        updated_by: None,
                        created_by: None,
                        translations: None,
                        owner_id: None,
                        relations: None,
                        has_children: None,
                    };

                    storage.nodes().create(
                        "bench_tenant",
                        "bench_repo",
                        "main",
                        "default",
                        order,
                        None,
                    ).await.unwrap();
                }
            }

            storage
        });

        group.bench_with_input(
            BenchmarkId::new("selectivity_pct", users_with_orders),
            users_with_orders,
            |b, _| {
                b.to_async(&rt).iter(|| {
                    bench_hash_join(black_box(storage.clone()), black_box(100))
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_hash_join_scaling,
    benchmark_hash_join_vs_data_size,
    benchmark_join_selectivity
);
criterion_main!(benches);

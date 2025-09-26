/*
* SPDX-FileCopyrightText: Copyright 2024 LG Electronics Inc.
* SPDX-License-Identifier: Apache-2.0
*/

//! Simple test to demonstrate StateManager model functionality

use common::monitoringserver::ContainerInfo;
use std::collections::HashMap;

// Import StateManager components from the library crate
use statemanager::state_machine::StateMachine;

/// Test the model state evaluation logic using actual StateManager implementation
fn test_model_state_evaluation() {
    println!("=== StateManager Model State Test ===");
    
    // Create a StateMachine instance for testing
    let mut state_machine = StateMachine::new();
    
    // Test Case 1: Mixed containers (running + dead) - should result in Dead state
    println!("Test Case 1: Mixed containers (running + dead) for test_model");
    
    // Container 1: Running
    let mut container1_state = HashMap::new();
    container1_state.insert("Status".to_string(), "running".to_string());
    container1_state.insert("Running".to_string(), "true".to_string());
    
    let mut container1_annotation = HashMap::new();
    container1_annotation.insert("model".to_string(), "test_model".to_string());
    
    let container1 = ContainerInfo {
        id: "container1".to_string(),
        names: vec!["model-test_model-container1".to_string()],
        image: "test:latest".to_string(),
        state: container1_state,
        config: HashMap::new(),
        annotation: container1_annotation,
        stats: HashMap::new(),
    };
    
    // Container 2: Dead
    let mut container2_state = HashMap::new();
    container2_state.insert("Status".to_string(), "dead".to_string());
    
    let mut container2_annotation = HashMap::new();
    container2_annotation.insert("model".to_string(), "test_model".to_string());
    
    let container2 = ContainerInfo {
        id: "container2".to_string(),
        names: vec!["model-test_model-container2".to_string()],
        image: "test:latest".to_string(),
        state: container2_state,
        config: HashMap::new(),
        annotation: container2_annotation,
        stats: HashMap::new(),
    };
    
    // Test Case 2: All containers paused - should result in Paused state
    println!("Test Case 2: All containers paused for paused_model");
    let mut container3_state = HashMap::new();
    container3_state.insert("Status".to_string(), "paused".to_string());
    
    let mut container3_annotation = HashMap::new();
    container3_annotation.insert("model".to_string(), "paused_model".to_string());
    
    let container3 = ContainerInfo {
        id: "container3".to_string(),
        names: vec!["model-paused_model-container1".to_string()],
        image: "test:latest".to_string(),
        state: container3_state,
        config: HashMap::new(),
        annotation: container3_annotation,
        stats: HashMap::new(),
    };
    
    // Test the actual state evaluation logic
    println!("\n=== Testing StateManager Logic ===");
    
    // Test test_model with mixed containers (running + dead)
    let test_model_containers = vec![&container1, &container2];
    let result1 = state_machine.process_model_state_update("test_model", &test_model_containers);
    
    println!("✓ Container 1 (running) mapped to test_model");
    println!("✓ Container 2 (dead) mapped to test_model");
    println!("Result for test_model: {:?}", result1);
    println!("Expected: Dead state (because one container is dead)");
    
    // Test paused_model with all containers paused  
    let paused_model_containers = vec![&container3];
    let result2 = state_machine.process_model_state_update("paused_model", &paused_model_containers);
    
    println!("\n✓ Container 3 (paused) mapped to paused_model");
    println!("Result for paused_model: {:?}", result2);
    println!("Expected: Paused state (because all containers are paused)");
    
    println!("\n=== Expected ETCD Operations ===");
    println!("  PUT /model/test_model/state = \"Dead\"");
    println!("  PUT /model/paused_model/state = \"Paused\"");
    
    // Validate results
    println!("\n=== Validation ===");
    if result1.is_success() {
        println!("✅ test_model state transition successful");
        match result1.new_state {
            4 => println!("✅ test_model correctly evaluated to Dead state"),
            _ => println!("❌ test_model state was {}, expected Dead (4)", result1.new_state),
        }
    } else {
        println!("❌ test_model state transition failed: {}", result1.message);
    }
    
    if result2.is_success() {
        println!("✅ paused_model state transition successful"); 
        match result2.new_state {
            2 => println!("✅ paused_model correctly evaluated to Paused state"),
            _ => println!("❌ paused_model state was {}, expected Paused (2)", result2.new_state),
        }
    } else {
        println!("❌ paused_model state transition failed: {}", result2.message);
    }
    
    println!("\n=== Test Completed ===");
}

fn main() {
    test_model_state_evaluation();
}
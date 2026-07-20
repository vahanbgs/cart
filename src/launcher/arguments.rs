use std::collections::HashMap;

use crate::api::piston::{Action, Argument, ArgumentValue, Os, Rule};

pub fn substitute(argument_string: &str, variables: &HashMap<&str, String>) -> String {
    let mut result = argument_string.to_owned();

    for (key, value) in variables {
        result = result.replace(&format!("${{{key}}}"), value);
    }

    result
}

pub fn evaluate_rules(rules: &[Rule]) -> bool {
    let mut allow = rules.is_empty();

    for rule in rules {
        let mut applies = true;

        if let Some(os) = &rule.os {
            applies &= match os {
                Os::Arch { .. } => true,
                Os::Name { name } => name.matches_current_platform(),
            };
        }

        applies &= rule.features.is_none();

        if applies {
            allow = rule.action == Action::Allow;
        }
    }

    allow
}

pub fn resolve(arguments: &[Argument], variables: &HashMap<&str, String>) -> Vec<String> {
    let mut processed_arguments = Vec::new();

    for argument in arguments {
        match argument {
            Argument::Simple(simple_argument) => {
                processed_arguments.push(substitute(simple_argument, variables))
            }
            Argument::Complex { rules, value } if evaluate_rules(rules) => match value {
                ArgumentValue::Simple(simple_argument) => {
                    processed_arguments.push(substitute(simple_argument, variables))
                }
                ArgumentValue::Multiple(arguments) => {
                    processed_arguments.extend(arguments.iter().map(|s| substitute(s, variables)))
                }
            },
            Argument::Complex { .. } => {}
        }
    }

    processed_arguments
}

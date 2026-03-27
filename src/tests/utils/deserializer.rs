#[cfg(test)]
mod tests {
    use crate::utils::deserializer::{camel_to_snake, deserialize_snake_case, to_snake_case};
    use serde::Deserialize;
    use toml::Value;
    use toml::map::Map;

    #[test]
    fn camel_to_snake_simple() {
        assert_eq!(camel_to_snake("CamelCase"), "camel_case");
        assert_eq!(camel_to_snake("simpleTest"), "simple_test");
        assert_eq!(camel_to_snake("already_snake"), "already_snake");
        assert_eq!(camel_to_snake("X"), "x");
        assert_eq!(camel_to_snake("ABTest"), "a_b_test");
    }

    #[test]
    fn to_snake_case_nested_table() {
        let mut inner_table = Map::new();
        inner_table.insert("InnerKey".into(), Value::String("value".into()));

        let mut outer_table = Map::new();
        outer_table.insert("OuterKey".into(), Value::Table(inner_table));

        let value = Value::Table(outer_table);

        // Expected snake_case
        let mut expected_inner = Map::new();
        expected_inner.insert("inner_key".into(), Value::String("value".into()));

        let mut expected_outer = Map::new();
        expected_outer.insert("outer_key".into(), Value::Table(expected_inner));

        let expected = Value::Table(expected_outer);

        let result = to_snake_case(value);
        assert_eq!(result, expected);
    }

    #[test]
    fn to_snake_case_array_of_tables() {
        let mut table1 = Map::new();
        table1.insert("CamelKey".into(), Value::Integer(1));

        let mut table2 = Map::new();
        table2.insert("AnotherKey".into(), Value::Integer(2));

        let value = Value::Array(vec![Value::Table(table1), Value::Table(table2)]);

        let mut expected_table1 = Map::new();
        expected_table1.insert("camel_key".into(), Value::Integer(1));

        let mut expected_table2 = Map::new();
        expected_table2.insert("another_key".into(), Value::Integer(2));

        let expected = Value::Array(vec![
            Value::Table(expected_table1),
            Value::Table(expected_table2),
        ]);

        let result = to_snake_case(value);
        assert_eq!(result, expected);
    }

    #[test]
    fn deserialize_snake_case_works_with_struct() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Config {
            some_value: i32,
            nested_table: Nested,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct Nested {
            inner_value: String,
        }

        let toml_str = r#"
            SomeValue = 42

            [NestedTable]
            InnerValue = "hello"
        "#;

        let value: Value = toml::from_str(toml_str).unwrap();
        let snake_value = deserialize_snake_case(value).unwrap();

        // Deserialize to struct
        let config: Config = snake_value.try_into().unwrap();

        assert_eq!(config.some_value, 42);
        assert_eq!(config.nested_table.inner_value, "hello");
    }

    #[test]
    fn to_snake_case_non_table_value() {
        let value = Value::String("unchanged".into());
        let result = to_snake_case(value.clone());
        assert_eq!(result, value);
    }
}

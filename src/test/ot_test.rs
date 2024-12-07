use operational_transform::OperationSeq;

#[test]
fn test_ot() {
    let mut initial_text = "hello world\r\n".to_string();
    let mut initial_ots = Vec::new();

    let mut first_ot = OperationSeq::default();
    let len = initial_text.len() as u64;
    first_ot.retain(3);
    first_ot.insert("!");
    first_ot.retain(len - 3);
    initial_ots.push(&first_ot);

    initial_text = first_ot.apply(&initial_text).unwrap();

    let mut ot = OperationSeq::default();
    ot.retain(3);
    ot.insert(" ");
    ot.retain(len - 3);
    ot = ot.transform(&first_ot).unwrap().0;

    initial_ots.push(&ot);
    initial_text = ot.apply(&initial_text).unwrap();

    assert_eq!(initial_text, "hello world\r\n! ");
}

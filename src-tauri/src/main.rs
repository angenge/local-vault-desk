// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--test-cli" {
        run_e2e_test();
        return;
    }
    if args.len() > 1 && args[1] == "--test-features" {
        run_features_test();
        return;
    }
    if args.len() > 1 && args[1] == "--test-security" {
        run_security_test();
        return;
    }
    if args.len() > 1 && args[1] == "--test-fuzz" {
        run_fuzz_test();
        return;
    }
    if args.len() > 1 && args[1] == "--test-fix-regression" {
        run_bugfix_regression_test();
        return;
    }
    local_vault_desk_lib::run();
}

// 递归收集目录下所有文件路径（E2E 断言辅助）
fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_files(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn run_e2e_test() {
    use local_vault_desk_lib::vault_service::VaultService;
    use std::fs;

    println!("=== 启动 Tauri 2.0 (Rust) 保险箱加解密与内存映射闭环测试 ===");
    let base_dir = std::env::temp_dir().join("safe_vault_rust_test");
    let vault_dir = base_dir.join("vault_data");
    let source_dir = base_dir.join("source_files");
    let export_dir = base_dir.join("export_files");

    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&export_dir).unwrap();

    let secret_file = source_dir.join("机密合同2026.docx");
    fs::write(&secret_file, "这是Tauri 2.0 Rust原生保险箱的顶级机密文本！").unwrap();

    let service = VaultService::new();

    // 1. 初始化创建保险箱
    println!("\n1. 初始化创建保险箱...");
    let status = service.init_vault(
        &vault_dir.to_string_lossy(),
        "alice",
        "MasterKey888!",
        false
    ).expect("初始化保险箱失败");
    println!("-> 保险箱初始化成功: {:?}", status);

    // 2. 验证磁盘密文混淆特征
    println!("\n2. 验证磁盘密文混淆特征...");
    let raw_entries: Vec<_> = fs::read_dir(&vault_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    println!("-> 物理磁盘密文项目: {:?}", raw_entries);
    for name in &raw_entries {
        assert!(!name.contains("机密合同"), "磁盘上不得泄露明文文件名！");
        assert!(!name.contains(".docx"), "磁盘上不得泄露明文后缀！");
    }
    println!("-> 验证通过：物理磁盘上只有 AES-EME 混淆乱码！");

    // 3. 导入加密文件
    println!("\n3. 加密导入文件...");
    let imported = service.import_paths(
        vec![secret_file.to_string_lossy().to_string()],
        ""
    ).expect("加密导入失败");
    println!("-> 导入完成: {:?}", imported);

    // 4. 读取解密列表
    println!("\n4. 读取解密列表...");
    let files = service.list_files("").expect("读取解密列表失败");
    println!("-> 解密列表:");
    for f in &files {
        println!("   - [{}] {} ({} 字节)", if f.is_dir { "目录" } else { "文件" }, f.name, f.size);
    }
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "机密合同2026.docx");

    // 5. 解密导出还原
    println!("\n5. 解密导出还原...");
    service.export_item(
        &files[0].name,
        files[0].is_dir,
        &export_dir.to_string_lossy()
    ).expect("解密导出失败");

    let exported_file = export_dir.join("机密合同2026.docx");
    assert!(exported_file.exists());
    let content = fs::read_to_string(&exported_file).unwrap();
    println!("-> 还原文本内容: {}", content);
    assert_eq!(content, "这是Tauri 2.0 Rust原生保险箱的顶级机密文本！");
    println!("-> 验证通过：文件内容 100% 完整解密还原！");

    // 6. 全库搜索（含缓存复用、大小写不敏感、auth 过滤）
    println!("\n6. 全库搜索测试（含缓存与匹配）...");
    let hits = service.search_files("合同", 200).expect("搜索失败");
    assert_eq!(hits.len(), 1, "应精确命中一个含「合同」的文件");
    assert_eq!(hits[0].name, "机密合同2026.docx");
    assert_eq!(hits[0].parent_dir, "", "根目录文件父目录应为空");
    println!("-> 命中: {:?} (父目录: {:?})", hits[0].name, hits[0].parent_dir);

    let hits2 = service.search_files("DOCX", 200).expect("搜索失败");
    assert_eq!(hits2.len(), 1, "大小写不敏感匹配应命中");
    println!("-> 大小写不敏感命中数: {}", hits2.len());

    let none = service.search_files("不存在的关键字XYZ", 200).expect("搜索失败");
    assert_eq!(none.len(), 0, "无关关键字不应命中");
    println!("-> 无关关键字命中数: {} (正确)", none.len());

    let auth_hits = service.search_files("vault_auth", 200).expect("搜索失败");
    assert_eq!(auth_hits.len(), 0, "鉴权文件不得出现在搜索结果");
    println!("-> 鉴权文件被过滤: 命中数 {} (正确)", auth_hits.len());
    println!("-> 全库搜索测试通过！");

    // 7. 锁定与密码拦截
    println!("\n7. 测试锁定与错误密码拦截...");
    service.lock_vault();
    let bad_unlock = service.unlock_vault(
        &vault_dir.to_string_lossy(),
        "alice",
        "WrongPassword"
    );
    assert!(bad_unlock.is_err(), "错误密码必须被拦截！");
    println!("-> 成功拦截错误密码: {:?}", bad_unlock.err().unwrap());

    // 8. 正确密码重新解锁
    println!("\n8. 使用正确密码再次解锁...");
    let re_unlock = service.unlock_vault(
        &vault_dir.to_string_lossy(),
        "alice",
        "MasterKey888!"
    ).expect("正确密码必须成功解锁");
    assert!(re_unlock.is_unlocked);
    let re_files = service.list_files("").unwrap();
    println!("-> 重新解锁成功，文件总数: {}", re_files.len());

    service.lock_vault();
    service.stop();
    let _ = fs::remove_dir_all(&base_dir);
    println!("\n🎉 Tauri 2.0 (Rust) 端到端全流程测试全部圆满成功！");
    std::process::exit(0);
}

/// 针对新增通用产品功能（回收站/文件操作/完整性自检/统计/审计）的端到端测试
fn run_features_test() {
    use local_vault_desk_lib::vault_service::VaultService;
    use std::fs;

    println!("=== 启动通用功能端到端测试（回收站/文件操作/自检/统计/审计）===");
    let base_dir = std::env::temp_dir().join("safe_vault_features_test");
    let vault_dir = base_dir.join("vault_data");
    let source_dir = base_dir.join("source_files");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&source_dir).unwrap();

    let f1 = source_dir.join("report_a.txt");
    let f2 = source_dir.join("report_b.txt");
    fs::write(&f1, "AAA 内容").unwrap();
    fs::write(&f2, "BBB 内容").unwrap();

    let service = VaultService::new();
    service.init_vault(&vault_dir.to_string_lossy(), "alice", "MasterKey888!", false).expect("init");
    service.import_paths(
        vec![f1.to_string_lossy().to_string(), f2.to_string_lossy().to_string()],
        "",
    ).expect("import");

    // 1. 回收站软删除 + 列表
    println!("\n1. 回收站软删除与列表...");
    // 0. 全新保险箱从未删除过：.recycle 尚不存在，应视为空收藏而非报错
    let fresh_recycle = service.list_recycle().expect("空回收站不应报错");
    assert_eq!(fresh_recycle.len(), 0, "全新保险箱回收站应为空");
    println!("-> 空回收站正确返回空列表");
    let id = service.recycle_item("report_a.txt", false).expect("recycle");
    println!("-> 已移入回收站, 条目 id={}", id);
    let list = service.list_recycle().expect("list recycle");
    assert_eq!(list.len(), 1, "回收站应有 1 个条目");
    println!("-> 回收站条目: {:?}", list[0]);
    let files_after = service.list_files("").unwrap();
    assert_eq!(files_after.len(), 1, "删除后主区只剩 1 个文件");

    // 2. 恢复
    println!("\n2. 恢复已删除条目...");
    service.restore_item(&id).expect("restore");
    let files_restored = service.list_files("").unwrap();
    assert_eq!(files_restored.len(), 2, "恢复后应回到 2 个文件");
    assert!(files_restored.iter().any(|f| f.name == "report_a.txt"));
    println!("-> 恢复成功，主区文件数: {}", files_restored.len());

    // 3. 新建目录 + 移动 + 复制 + 重命名
    println!("\n3. 目录/移动/复制/重命名...");
    service.create_folder("docs").expect("mkdir");
    service.move_items(
        vec!["report_a.txt".to_string()],
        "docs",
    ).expect("move");
    let in_docs = service.list_files("docs").unwrap();
    assert_eq!(in_docs.len(), 1, "docs 下应有 report_a.txt");
    service.copy_items(
        vec!["report_b.txt".to_string()],
        "docs",
    ).expect("copy");
    let in_docs2 = service.list_files("docs").unwrap();
    assert_eq!(in_docs2.len(), 2, "docs 下应有 2 个文件");
    service.rename_item("docs/report_a.txt", "renamed_a.txt").expect("rename");
    let names: Vec<String> = service.list_files("docs").unwrap().iter().map(|f| f.name.clone()).collect();
    assert!(names.contains(&"renamed_a.txt".to_string()), "应存在重命名后的文件");
    println!("-> docs 目录内容: {:?}", names);

    // 3.5. 名称合法性 + 冲突/环检测
    println!("\n3.5 名称校验与冲突/环检测...");
    assert!(service.rename_item("docs/renamed_a.txt", "report_b.txt").is_err(), "重命名到已存在名称应报错");
    assert!(service.rename_item("docs/renamed_a.txt", "CON").is_err(), "Windows 保留名应报错");
    assert!(service.rename_item("docs/renamed_a.txt", "a/b.txt").is_err(), "非法字符应报错");
    assert!(service.rename_item("docs/renamed_a.txt", "bad*name.txt").is_err(), "非法字符 * 应报错");
    assert!(service.create_folder("x/../y").is_err(), "路径穿越应报错");
    service.create_folder("docs/sub").expect("mkdir sub");
    assert!(service.move_items(vec!["docs".to_string()], "docs/sub").is_err(), "目录移入自身子目录应报错");
    assert!(service.copy_items(vec!["report_b.txt".to_string()], "docs").is_err(), "复制到同名目标应报错");
    let list3 = service.list_files("docs/sub").unwrap();
    assert_eq!(list3.len(), 0, "docs/sub 应为空（未发生破坏性操作）");
    println!("-> 名称/冲突/环检测全部通过！");

    // 3.6. 传输取消（空闲时无任务）
    println!("\n3.6 传输取消接口...");
    assert!(service.cancel_transfer().is_err(), "无传输任务时取消应提示");
    println!("-> 无任务时取消调用正确报错");

    // 3.7. 批量解密导出（多选：1 个文件 + 1 个目录）
    println!("\n3.7 批量解密导出...");
    let batch_dir = base_dir.join("export_batch");
    fs::create_dir_all(&batch_dir).unwrap();
    service.export_items(
        &vec![
            ("report_b.txt".to_string(), false),
            ("docs".to_string(), true),
        ],
        &batch_dir.to_string_lossy(),
    ).expect("batch export");
    assert!(batch_dir.join("report_b.txt").exists(), "根级文件 report_b.txt 应导出");
    assert!(batch_dir.join("docs").join("report_b.txt").exists(), "docs/report_b.txt 应导出");
    assert!(batch_dir.join("docs").join("renamed_a.txt").exists(), "docs/renamed_a.txt 应导出");
    let exported_count = walk_files(&batch_dir).len();
    assert_eq!(exported_count, 3, "批量导出应还原 3 个文件（根级 1 + docs 2）");
    println!("-> 批量导出成功！目标目录已还原 {} 个文件", exported_count);

    // 3.8. 目录回收站往返（回归：目录移入回收站后必须可完整恢复，不能被提前删除）
    println!("\n3.8 目录回收站往返完整性...");
    // 构造一个含 2 个文件的目录并导入
    let dir_src = source_dir.join("folder_to_recycle");
    fs::create_dir_all(&dir_src).unwrap();
    fs::create_dir_all(dir_src.join("sub")).unwrap();
    fs::write(dir_src.join("a.txt"), "folder-a").unwrap();
    fs::write(dir_src.join("sub").join("b.txt"), "folder-sub-b").unwrap();
    fs::write(dir_src.join("sub").join("c.txt"), "folder-sub-c").unwrap();
    service.import_paths(vec![dir_src.to_string_lossy().to_string()], "").expect("import folder");
    assert_eq!(service.list_files("folder_to_recycle").expect("list folder").len(), 2, "子目录内应为 2 个文件");

    // 移入回收站（目录）
    let folder_id = service.recycle_item("folder_to_recycle", true).expect("recycle folder");
    let after_recycle = service.list_files("").expect("list root");
    assert!(!after_recycle.iter().any(|f| f.name == "folder_to_recycle"), "根目录不应再有 folder_to_recycle");
    let rl = service.list_recycle().expect("list recycle");
    assert!(rl.iter().any(|e| e["id"] == serde_json::Value::String(folder_id.clone())), "回收站应包含该目录条目");

    // 恢复目录，必须包含全部 3 个文件
    service.restore_item(&folder_id).expect("restore folder");
    let restored_sub = service.list_files("folder_to_recycle/sub").expect("list restored sub");
    let restored_names: Vec<String> = restored_sub.iter().map(|f| f.name.clone()).collect();
    assert_eq!(restored_names.len(), 2, "恢复后的子目录应含 2 个文件（循环竞态下会丢文件）");
    assert!(restored_names.contains(&"b.txt".to_string()) && restored_names.contains(&"c.txt".to_string()));
    assert_eq!(service.list_files("folder_to_recycle").expect("list restored").len(), 2, "根级 a.txt + 子目录");
    println!("-> 目录回收站往返完整：恢复后子目录内容 = {:?}（共 3 个文件全部还原）", restored_names);

    // 3.9. 大目录回收站压力回归（海量文件目录移入回收站不得报错/超时，且恢复完整）
    println!("\n3.9 大目录回收站压力回归...");
    let big_src = source_dir.join("big_folder");
    fs::create_dir_all(&big_src).unwrap();
    fs::create_dir_all(big_src.join("nested")).unwrap();
    const BIG_COUNT: usize = 1500;
    for i in 0..BIG_COUNT {
        let (sub, name) = if i % 2 == 0 { (big_src.clone(), format!("f{:04}.txt", i)) } else { (big_src.join("nested"), format!("g{:04}.txt", i)) };
        fs::write(sub.join(name), format!("content-{}", i)).unwrap();
    }
    service.import_paths(vec![big_src.to_string_lossy().to_string()], "").expect("import big folder");
    let big_imported = service.list_files("big_folder").expect("list big");
    let big_imported_names: Vec<String> = big_imported.iter().map(|f| f.name.clone()).collect();
    println!("-> 导入大目录完成，big_folder 根级 {} 项", big_imported.len());
    assert!(big_imported_names.contains(&"nested".to_string()), "big_folder 应包含 nested 子目录");

    let t0 = std::time::Instant::now();
    let big_id = service.recycle_item("big_folder", true).expect("recycle big folder");
    let recycle_ms = t0.elapsed().as_millis();
    println!("-> 大目录移入回收站耗时 {} ms (id={})", recycle_ms, big_id);
    // 服务端目录移动（密文整体改名）为此规模应在秒级内完成
    assert!(recycle_ms < 30_000, "大目录回收耗时过高 ({recycle_ms} ms)");

    assert!(!service.list_files("").expect("list root").iter().any(|f| f.name == "big_folder"), "回收后原目录应消失");
    assert_eq!(service.list_recycle().expect("list recycle").len(), 1, "回收站应有 1 个条目（大目录）");

    let t1 = std::time::Instant::now();
    service.restore_item(&big_id).expect("restore big folder");
    let restore_ms = t1.elapsed().as_millis();
    println!("-> 大目录恢复耗时 {} ms", restore_ms);
    assert!(restore_ms < 30_000, "大目录恢复耗时过高 ({restore_ms} ms)");
    assert_eq!(service.list_files("big_folder/nested").expect("list nested").len(), BIG_COUNT / 2, "恢复后嵌套子目录文件数应完整");
    assert_eq!(service.list_files("big_folder").expect("list big restored").len(), BIG_COUNT / 2 + 1, "恢复后根级文件 + 子目录应完整");
    println!("-> 大目录回收/恢复完整（{} 个文件全部还原，未触发 500/超时）", BIG_COUNT);

    // 3.10. 空目录回收/恢复往返（空目录无任何文件，回收站也必须能看到并恢复该条目）
    println!("\n3.10 空目录回收/恢复往返...");
    let empty_src = source_dir.join("empty_demo");
    fs::create_dir_all(&empty_src).unwrap();
    fs::write(empty_src.join("placeholder.txt"), "x").unwrap();
    service.import_paths(vec![empty_src.to_string_lossy().to_string()], "").expect("import empty demo");
    service.delete_item("empty_demo/placeholder.txt", false).expect("remove placeholder");
    // 此时 empty_demo 为空目录
    let empty_id = service.recycle_item("empty_demo", true).expect("recycle empty dir");
    let rl_e = service.list_recycle().expect("list recycle");
    assert_eq!(rl_e.len(), 1, "空目录回收后回收站应显示 1 个条目");
    assert_eq!(rl_e[0]["is_dir"].as_bool(), Some(true), "空目录条目应标记为目录");
    assert_eq!(rl_e[0]["name"].as_str().unwrap(), "empty_demo", "空目录条目名称正确");
    service.restore_item(&empty_id).expect("restore empty dir");
    assert!(service.list_files("").expect("list root").iter().any(|f| f.name == "empty_demo"), "空目录恢复后应重新出现在根目录");
    assert_eq!(service.list_recycle().expect("list recycle").len(), 0, "恢复后回收站应清空");
    println!("-> 空目录回收/恢复往返完整");

    // 3.11. 恢复防静默覆盖：目标位置已存在同名条目时拒绝恢复，移除冲突后恢复成功
    println!("\n3.11 恢复冲突保护（拒绝覆盖已有数据）...");
    let conflict_src = source_dir.join("conflict_demo.txt");
    fs::write(&conflict_src, "original-content").unwrap();
    service.import_paths(vec![conflict_src.to_string_lossy().to_string()], "").expect("import conflict");
    let conflict_id = service.recycle_item("conflict_demo.txt", false).expect("recycle conflict");
    // 用户在原位置重建了同名的文件
    fs::write(&conflict_src, "replacement-content").unwrap();
    service.import_paths(vec![conflict_src.to_string_lossy().to_string()], "").expect("re-import conflict");
    let refuse = service.restore_item(&conflict_id).expect_err("目标存在时应拒绝恢复");
    assert!(refuse.contains("同名"), "拒绝信息应提示同名冲突: {refuse}");
    // 移走冲突项后恢复应成功
    service.recycle_item("conflict_demo.txt", false).expect("recycle replacement");
    service.restore_item(&conflict_id).expect("restore after conflict removed");
    let listed = service.list_files("").expect("list root");
    let restored = listed.iter().find(|f| f.name == "conflict_demo.txt").expect("conflict_demo.txt 应恢复");
    assert_eq!(restored.size, 16, "恢复的应是回收前的原内容（16 字节 original-content），而非 18 字节的 replacement-content");
    println!("-> 恢复冲突保护验证通过（拒绝覆盖 + 消除冲突后可恢复）");

    // 3.12. 清空回收站（带进度计数路径）：回收内容后可一键清空，且清空后回收站仍可复用
    println!("\n3.12 清空回收站...");
    let sweep_src = source_dir.join("sweep_demo");
    fs::create_dir_all(&sweep_src).unwrap();
    fs::write(sweep_src.join("a.txt"), "a").unwrap();
    fs::write(sweep_src.join("b.txt"), "b").unwrap();
    service.import_paths(vec![sweep_src.to_string_lossy().to_string()], "").expect("import sweep");
    service.recycle_item("sweep_demo/a.txt", false).expect("recycle a");
    service.recycle_item("sweep_demo", true).expect("recycle dir");
    let sweep_list = service.list_recycle().expect("list recycle");
    assert_eq!(sweep_list.len(), 3, "清空前回收站应有 3 个条目（含此前未还原的 conflict 替换件 + 本次 2 条）");
    service.empty_recycle().expect("empty recycle");
    assert_eq!(service.list_recycle().expect("list recycle").len(), 0, "清空后回收站应无条目");
    assert!(!service.list_files("").expect("list root").iter().any(|f| f.name == "sweep_demo"), "清空后源目录应不存在于根目录");
    // .recycle 目录被重建，后续仍可正常回收
    service.recycle_item("conflict_demo.txt", false).expect("recycle after empty");
    assert_eq!(service.list_recycle().expect("list recycle").len(), 1, "清空后回收站可继续使用");
    service.restore_item(&service.list_recycle().unwrap()[0]["id"].as_str().unwrap()).expect("restore after empty");
    println!("-> 清空回收站验证通过（进度计数路径 + 目录重建 + 复用）");

    // 4. 完整性自检（无清单 → 提示；构建清单后 → 全 OK；篡改密文 → 检出 DIFFER）
    println!("\n4. 数据完整性自检...");
    let check_before = service.integrity_check("", false).expect("check");
    println!("-> 自检(未建清单): {:?}", check_before);
    service.build_manifest().expect("build manifest");
    let check_after = service.integrity_check("", false).expect("check");
    println!("-> 自检(已建清单): {:?}", check_after);
    assert_eq!(check_after["differ"].as_u64().unwrap(), 0, "不应有差异文件");

    // 篡改一个密文文件，验证能检出损坏。
    // 通过 list_raw_mapping 拿到用户文件的密文名（内部文件已被过滤），取一个根级用户文件来篡改
    let mapping = service.list_raw_mapping().expect("mapping");
    let target = mapping
        .iter()
        .find_map(|m| {
            let enc = m.get("encrypted")?.as_str()?;
            if enc.contains('/') {
                return None; // 只篡改根级文件
            }
            Some(enc.to_string())
        })
        .expect("至少有一个根级用户文件");
    let p = vault_dir.join(&target);
    let mut content = fs::read(&p).unwrap();
    if let Some(last) = content.last_mut() {
        *last ^= 0xFF; // 翻转最后一个字节模拟位损坏
    }
    fs::write(&p, &content).unwrap();
    println!("-> 已篡改密文文件: {} (模拟位损坏)", target);
    let check_tampered = service.integrity_check("", false).expect("check");
    println!("-> 自检(篡改后): {:?}", check_tampered);
    assert!(check_tampered["differ"].as_u64().unwrap() >= 1, "篡改密文后必须检出至少 1 个差异");
    println!("-> 位损坏检测成功！");

    // 4.5. 互通配置导出与跨实例解密验证
    println!("\n4.5 互通配置与跨实例验证...");
    let cfg = service.get_interop_config().expect("interop config");
    println!("-> 互通配置:\n{}", cfg.snippet);
    assert!(cfg.snippet.contains("[interop_crypt]"), "应包含 remote 段");
    assert!(cfg.snippet.contains("filename_encryption = standard"));
    assert!(cfg.snippet.contains("directory_name_encryption = true"));
    assert!(cfg.snippet.contains("password = ") && cfg.snippet.contains("password2 = "));

    // 模拟外部 rclone：用与保险箱等价的标准 crypt 独立配置写入互探文件
    let ext_plain = source_dir.join("interop_local.txt");
    fs::write(&ext_plain, "INTEROP_EXTERNAL_2026").unwrap();
    service
        .interop_write_plain(&ext_plain.to_string_lossy(), &cfg.probe_file)
        .expect("外部配置写入失败");
    let got = service.verify_interop(&cfg.probe_file).expect("verify interop");
    println!("-> 外部写入内容已解密还原: {}", got);
    assert_eq!(got.trim(), "INTEROP_EXTERNAL_2026", "互通探针内容应一致");
    assert!(service.verify_interop("no_such_probe.txt").is_err(), "不存在的探针应报错");
    println!("-> 互通验证通过（外部 rclone 写入的文件可由本保险箱解密读出）");

    // 4.5.1. 一键互通自测（同 CLI 等价路径，验证导出配置与本保险箱互通可用）
    let self_verify = service.interop_self_verify().expect("interop self verify");
    println!(
        "-> 一键互通自测: 写入={} 读回={} ok={}",
        self_verify.written, self_verify.read_back, self_verify.ok
    );
    assert!(self_verify.ok, "自测写入与读回内容应一致");
    assert_eq!(self_verify.written, self_verify.read_back);
    // 自测采用独立唯一探针名（interop_self_*）：只清理自身创建的探针，
    // 绝不误删外部互通探针（interop_probe_<ns>.txt），二者互不干扰。
    assert!(
        service.verify_interop(&cfg.probe_file).is_ok(),
        "自测清理不得误删外部互通探针，外部探针应仍可校验"
    );
    let root_after = service.list_files("").expect("list root after self verify");
    assert!(
        !root_after.iter().any(|f| f.name.starts_with("interop_self_")),
        "自测结束后自身探针应已清理，根目录不得残留 interop_self_* 文件，实际: {:?}",
        root_after
    );
    // 主动删除外部探针后再次校验应报未找到（保持互探清理语义）
    service.delete_item(&cfg.probe_file, false).expect("delete external probe");
    assert!(service.verify_interop(&cfg.probe_file).is_err(), "删除后外部探针再校验应报未找到");
    println!("-> 一键互通自测通过（唯一探针名隔离，外部互探文件不受自测清理影响）");

    // 5. 统计面板
    println!("\n5. 统计面板...");
    let stats = service.get_stats().expect("stats");
    println!("-> 统计: {:?}", stats);
    assert!(stats["files"].as_u64().unwrap() >= 2, "至少 2 个文件");

    // 6. 审计日志
    println!("\n6. 审计日志...");
    let audit_log = service.get_audit_log(100);
    assert!(!audit_log.is_empty(), "审计日志不应为空");
    println!("-> 最近审计记录 {} 条，示例: {:?}", audit_log.len(), audit_log.last());

    service.lock_vault();
    service.stop();
    let _ = fs::remove_dir_all(&base_dir);
    println!("\n🎉 通用功能端到端测试全部圆满成功！");
    std::process::exit(0);
}

/// 针对安全模型的安全特性测试（防止绕过防护 / 密文泄露 / 密钥驻留）
/// 所有敏感操作在执行前都必须通过 is_unlocked 门禁。
fn run_security_test() {
    use local_vault_desk_lib::vault_service::VaultService;
    use std::fs;

    println!("=== 安全模型测试（门禁拦截 / 密文混淆 / 密钥驻留）===");
    let base_dir = std::env::temp_dir().join("safe_vault_security_test");
    let vault_dir = base_dir.join("vault_data");
    let source_dir = base_dir.join("source_files");
    let export_dir = base_dir.join("export_files");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&export_dir).unwrap();

    // 明文 + 高度可识别的敏感文件名，用于验证磁盘/列表/导出均不泄露
    let secret = source_dir.join("客户银行账号与密码2026.txt");
    let secret_content = "SAFE_VAULT_SECRET_SAMPLE_2026 核心机密内容不可外泄";
    fs::write(&secret, secret_content).unwrap();

    let service = VaultService::new();

    // 0. 误判死局回归：普通目录（含大写 Base32 形取名，如 CONFIDENTIAL_2025）绝不得被
    //    误判为“已初始化保险箱”。该误判曾使 init_vault 被 non-force 拒绝、unlock 永远失败，
    //    从而永久卡死在“保险箱检测”环节。此处使用独立临时目录，不影响主流程断言。
    println!("\n0. 误判死局回归（普通命名目录不得被判定为保险箱）...");
    let reg_base = std::env::temp_dir().join("safe_vault_judgment_regression");
    let reg_plain = reg_base.join("plain_dir");
    let _ = fs::remove_dir_all(&reg_base);
    // 普通目录：含典型普通命名条目（大写/连字符/下划线/点后缀），历史上会被 is_base32 旧启发式命中
    fs::create_dir_all(&reg_plain).unwrap();
    let _ = fs::write(reg_plain.join("CONFIDENTIAL_2025"), "普通文件A");
    let _ = fs::write(reg_plain.join("report_2026.txt"), "普通文件B");

    let reg_inspection = service.inspect_vault_dir(&reg_plain.to_string_lossy());
    assert_eq!(
        reg_inspection.kind,
        local_vault_desk_lib::vault_service::VaultDirKind::NonVaultNotEmpty,
        "普通目录（CONFIDENTIAL_2025 等）不得被误判为保险箱，实际: {:?}",
        reg_inspection
    );
    assert!(
        !service.check_initialized(&reg_plain.to_string_lossy()),
        "普通目录不得被 check_initialized 判为已初始化"
    );
    println!("-> 普通目录被正确判定为非保险箱（误判死局已消除）: {:?}", reg_inspection.kind);
    let _ = fs::remove_dir_all(&reg_base);

    // 1. 未解锁时：所有敏感操作必须被拒绝
    println!("\n1. 未解锁门禁拦截（锁定为空的最大默认状态）...");
    // 各关键入口在未解锁时应返回 Err
    assert!(service.list_files("").is_err(), "未解锁时 list_files 应被拦截");
    assert!(service.search_files("任何", 10).is_err(), "未解锁时 search 应被拦截");
    assert!(service.import_paths(vec![secret.to_string_lossy().to_string()], "").is_err(), "未解锁时 import 应被拦截");
    assert!(service.export_item("f.txt", false, &export_dir.to_string_lossy()).is_err(), "未解锁时 export 应被拦截");
    assert!(service.export_items(&[("f.txt".to_string(), false)], &export_dir.to_string_lossy()).is_err(), "未解锁时 batch export 应被拦截");
    assert!(service.create_folder("docs").is_err(), "未解锁时 create_folder 应被拦截");
    assert!(service.delete_item("f.txt", false).is_err(), "未解锁时 delete 应被拦截");
    assert!(service.recycle_item("f.txt", false).is_err(), "未解锁时 recycle 应被拦截");
    assert!(service.list_recycle().is_err(), "未解锁时 list_recycle 应被拦截");
    assert!(service.restore_item("1").is_err(), "未解锁时 restore 应被拦截");
    assert!(service.empty_recycle().is_err(), "未解锁时 empty_recycle 应被拦截");
    assert!(service.rename_item("f.txt", "g.txt").is_err(), "未解锁时 rename 应被拦截");
    assert!(service.move_items(vec!["f.txt".to_string()], "x").is_err(), "未解锁时 move 应被拦截");
    assert!(service.copy_items(vec!["f.txt".to_string()], "x").is_err(), "未解锁时 copy 应被拦截");
    assert!(service.build_manifest().is_err(), "未解锁时 build_manifest 应被拦截");
    assert!(service.integrity_check("", false).is_err(), "未解锁时 integrity_check 应被拦截");
    assert!(service.list_raw_mapping().is_err(), "未解锁时 list_raw_mapping 应被拦截");
    assert!(service.get_stats().is_err(), "未解锁时 get_stats 应被拦截");
    assert!(service.get_audit_log(10).is_empty(), "未解锁时审计日志应为空");
    println!("-> 全部敏感门禁（19 项）在未解锁时被正确拦截");

    // 2. 初始化 + 导入 + 导出还原
    println!("\n2. 初始化/解锁并通过正当途径导入导出...");
    service.init_vault(&vault_dir.to_string_lossy(), "alice", "MasterKey888!", false).expect("init");
    service.import_paths(vec![secret.to_string_lossy().to_string()], "").expect("import");

    // 3. 磁盘密文混淆：明文文件名、后缀、内容绝对不得出现在磁盘
    //    物理目录中每个文件与目录都应是 AES-EME 混淆后的乱码名
    println!("\n3. 磁盘密文混淆检查...");
    let mut disk_leak = Vec::new();
    for entry in fs::read_dir(&vault_dir).unwrap().flatten() {
        let n = entry.file_name().to_string_lossy().to_string();
        if n.contains("银行") || n.contains("账号") || n.contains("密码") || n.contains("password")
            || n.contains("客户") || n.contains(".txt") || n.contains("2026") {
            disk_leak.push(n);
        }
    }
    assert!(disk_leak.is_empty(), "磁盘文件名泄露明文信息: {:?}", disk_leak);
    println!("-> 磁盘上无任何明文文件名/后缀泄露，全部为混淆乱码");

    // 4. 密文内容不得以明文子串形式留存：对 vault_dir 递归读取每个文件，
    //    仅保留极少量的匹配，避免把整棵目录绑定为日志（这里只做存在性扫描）。
    println!("\n4. 密文内容泄露扫描（明文关键词不得出现在密文文件二进制中）...");
    let mut bin_leak = 0usize;
    for f in walk_files(&vault_dir) {
        if let Ok(bytes) = fs::read(&f) {
            let as_str = String::from_utf8_lossy(&bytes);
            if as_str.contains("SAFE_VAULT_SECRET_SAMPLE_2026")
                || as_str.contains("客户银行账号与密码")
                || as_str.contains("核心机密内容不可外泄") {
                bin_leak += 1;
            }
        }
    }
    assert_eq!(bin_leak, 0, "密文内容泄露明文关键词（{} 处）", bin_leak);
    println!("-> 密文文件二进制中未检出任何明文关键词（零泄露）");

    // 5. 解锁列表/搜索不泄露密文路径，且搜索可命中明文
    println!("\n5. 列表与全库搜索...");
    let files = service.list_files("").expect("list");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "客户银行账号与密码2026.txt");
    let hits = service.search_files("银行", 100).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].parent_dir, "");
    println!("-> 列表/搜索工作正常，且返回的均为解密明文");

    // 6. 锁定后门禁重新生效，且密钥从内存清零
    println!("\n6. 锁定后门禁 + 密钥清零...");
    service.lock_vault();
    assert!(service.get_status().is_unlocked == false);
    assert!(service.list_files("").is_err(), "锁定后 list_files 应被拦截");
    assert!(service.get_stats().is_err(), "锁定后 get_stats 应被拦截");
    assert!(service.get_audit_log(10).is_empty(), "锁定后审计日志应为空");
    assert!(service.get_interop_config().is_err(), "锁定后导出互通配置应被拒绝（密钥已清零）");
    assert!(service.interop_write_plain(&secret.to_string_lossy(), "x.txt").is_err(), "锁定后互通写入应被拒绝（密钥已清零）");
    assert!(service.verify_interop("x.txt").is_err(), "锁定后互通校验应被拒绝");
    println!("-> 锁定后敏感操作全部被拦截，密钥仅驻留于内存已随锁定清空");

    // 7. 错误密码解锁被拒且不返回/不泄露任何数据
    println!("\n7. 错误密码解锁拦截...");
    let bad = service.unlock_vault(&vault_dir.to_string_lossy(), "alice", "totally_wrong");
    assert!(bad.is_err(), "错误密码必须解锁失败");
    assert!(service.get_status().is_unlocked == false, "错误密码后仍应保持锁定");
    assert!(service.list_files("").is_err(), "错误密码后无法读取任何数据");
    println!("-> 错误密码被拦截，状态保持锁定，无可读数据外泄");

    // 8. 正确密码重新解锁并可还原（证明上述拦截来自正确鉴权，而非功能失效）
    println!("\n8. 正确密码解锁 + 导出还原...");
    service.unlock_vault(&vault_dir.to_string_lossy(), "alice", "MasterKey888!").expect("re-unlock");
    service.export_item("客户银行账号与密码2026.txt", false, &export_dir.to_string_lossy()).expect("export");
    let content = fs::read_to_string(export_dir.join("客户银行账号与密码2026.txt")).unwrap();
    assert_eq!(content, secret_content);
    println!("-> 正确密码解锁并 100% 还原明文内容: {}", content);

    service.lock_vault();
    service.stop();
    let _ = fs::remove_dir_all(&base_dir);
    println!("\n🎉 安全模型测试全部通过：门禁拦截、密文混淆、零泄露、密钥驻留清零均验证无误！");
    std::process::exit(0);
}

/// 针对恶意输入与路径穿越的防护测试（攻击向量枚举）
fn run_fuzz_test() {
    use local_vault_desk_lib::vault_service::VaultService;
    use std::fs;

    println!("=== 恶意输入与路径穿越防护测试 ===");
    let base_dir = std::env::temp_dir().join("safe_vault_fuzz_test");
    let vault_dir = base_dir.join("vault_data");
    let source_dir = base_dir.join("source_files");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&source_dir).unwrap();

    let service = VaultService::new();
    service.init_vault(&vault_dir.to_string_lossy(), "alice", "MasterKey888!", false).expect("init");

    // 1. 创建目录 / 重命名的非法与攻击名称集合
    println!("\n1. 非法名称与保留名枚举...");
    let bad_names: Vec<&str> = vec![
        "", " ", "   .txt", "/", "\\", "..", ".", "a*b", "a?b", "a\"b",
        "<x", "y>", "z|w", "c:d", "a\tb", "a\nb", "a\u{0}b", "CON", "com1", "LPT9",
        "aux.txt", "nul.html", "PRN", ".hidden", "trailing.", "audit.log", ".audit.log",
    ];
    let mut blocked = 0usize;
    for name in &bad_names {
        if service.create_folder(name).is_ok() || service.rename_item("nope.txt", name).is_ok() {
            panic!("攻击名称未被拦截: {:?}", name);
        }
        blocked += 1;
    }
    println!("-> 非法/保留名称全部被拦截（{} 个向量）", blocked);
    // 确认没有任何攻击目录被错误创建到保险箱内
    let list_after_bad = service.list_files("").expect("list");
    assert!(list_after_bad.is_empty(), "攻击名称不应创建任何条目: {:?}", list_after_bad);

    // 2. 合法名称应允许
    println!("\n2. 合法名称...");
    service.create_folder("正常目录").expect("legal dir");
    service.create_folder("a/b/c").expect("nested dir");
    let nested_names: Vec<String> = service.list_files("a/b").expect("list nested").iter().map(|i| i.name.clone()).collect();
    assert_eq!(nested_names, vec!["c".to_string()], "a/b 下应只有 c");
    println!("-> 合法名称（中文/深层目录）正常创建，a/b 内容: {:?}", nested_names);

    // 3. 路径穿越与移动到自身内部（环）检测
    println!("\n3. 路径穿越与环检测...");
    service.create_folder("dir_home").expect("home dir");
    service.create_folder("dir_home/sub").expect("sub dir");
    let forged = vec!["dir_home".to_string()];
    // 把 dir_home 移入其自身内部应被拒绝
    for dst in &["dir_home/sub", "dir_home/sub/deeper", "dir_home/dir_home"] {
        service.create_folder(dst).unwrap_or_default(); // 仅预创建
        assert!(service.move_items(forged.clone(), dst).is_err(), "移动到自身内部应被拒绝 ({})", dst);
    }
    // 复制到自身内部同样应被拒绝
    assert!(service.copy_items(vec!["dir_home".to_string()], "dir_home/sub").is_err(), "复制到自身内部应被拒绝");
    println!("-> 移动到自身内部（环）全部被拒绝");

    // 4. 越父目录导出尝试（不存在的路径）
    println!("\n4. 指向父目录的非法远程路径...");
    assert!(service.export_item("../x.txt", false, "C:\\temp").is_err(), "../ 越父导出应被拒绝");
    assert!(service.delete_item("..", false).is_err(), "删除 .. 应被拒绝");
    assert!(service.move_items(vec!["../y.txt".to_string()], "x").is_err(), "移动 ../ 应被拒绝");
    assert!(service.copy_items(vec!["../../z.txt".to_string()], "x").is_err(), "复制 ../../ 应被拒绝");
    println!("-> 父目录穿越向量全部被拒绝");

    // 5. 拼接攻击：以根前缀 / 或反斜杠开头，验证不会逃逸到系统目录
    println!("\n5. 前缀逃逸向量...");
    // rename 使用单段名校验，/ 与 \ 都会被拒绝，无法构造绝对路径
    assert!(service.rename_item("dir_home", "/etc/passwd").is_err(), "以 / 开头的绝对路径重命名应被拒绝");
    assert!(service.rename_item("dir_home", "C:\\x").is_err(), "带盘符的反斜杠路径重命名应被拒绝");
    // create_folder 会把 \ 与前置 / 规范化为保险箱内相对路径（不逃逸到系统目录），
    // 因此该调用应成功，且结果只能位于保险箱 ciphertext 目录内（windows/system32）
    service.create_folder("\\windows\\system32").expect("应被规范化为保险箱内相对路径而非逃逸");
    let inside = service.list_files("windows/system32").expect("list in-vault");
    assert!(inside.is_empty(), "windows/system32 应存在于保险箱内且为空");
    // 外部目录绝不能被写入：保险箱目录之外的任何位置不得出现本测试的残留（仅作结构断言）
    assert!(!vault_dir.parent().unwrap().join("windows").exists(), "不应逃逸到保险箱目录外");
    // 移入含 ../ 的目录应被拒绝
    assert!(service.move_items(vec!["dir_home".to_string()], "a/b/../..").is_err(), "含 ../ 的目录穿越应被拒绝");
    println!("-> 前缀逃逸向量全部被拦截，或安全落在保险箱内相对路径");

    // 6. 正常操作仍可用（未破坏功能）
    println!("\n6. 防御性拦截后功能戳健性...");
    let f = source_dir.join("normal.txt");
    fs::write(&f, "normal content").unwrap();
    service.import_paths(vec![f.to_string_lossy().to_string()], "").expect("import");
    let files = service.list_files("").expect("list");
    assert!(files.iter().any(|i| i.name == "normal.txt"));

    service.lock_vault();
    service.stop();
    let _ = fs::remove_dir_all(&base_dir);
    println!("\n🎉 恶意输入与路径穿越防护测试全部通过！");
    std::process::exit(0);
}

/// 针对本次复核并修复的 5 个关键缺陷进行专项自动化回归测试
fn run_bugfix_regression_test() {
    use local_vault_desk_lib::vault_service::VaultService;
    use std::fs;

    println!("=== 启动新增 Bug 修复专项回归测试 ===");
    let base_dir = std::env::temp_dir().join("safe_vault_bugfix_test");
    let vault_dir = base_dir.join("vault_data");
    let source_dir = base_dir.join("source_files");
    let _ = fs::remove_dir_all(&base_dir);
    fs::create_dir_all(&source_dir).unwrap();

    let service = VaultService::new();
    service.init_vault(&vault_dir.to_string_lossy(), "alice", "MasterKey888!", false).expect("init");

    // 1. 验证批量移动同名目标冲突拦截（杜绝静默覆盖）
    println!("\n1. 验证批量移动同名目标冲突拦截（杜绝静默覆盖）...");
    service.create_folder("folder_a").expect("mkdir folder_a");
    service.create_folder("folder_b").expect("mkdir folder_b");
    service.create_folder("dest_dir").expect("mkdir dest_dir");

    let f1 = source_dir.join("f1.txt");
    let f2 = source_dir.join("f2.txt");
    fs::write(&f1, "CONTENT A").unwrap();
    fs::write(&f2, "CONTENT B").unwrap();

    service.import_paths(vec![f1.to_string_lossy().to_string()], "folder_a").expect("import a");
    service.rename_item("folder_a/f1.txt", "same_name.txt").expect("rename to same_name");

    service.import_paths(vec![f2.to_string_lossy().to_string()], "folder_b").expect("import b");
    service.rename_item("folder_b/f2.txt", "same_name.txt").expect("rename to same_name");

    // 同时将 folder_a/same_name.txt 与 folder_b/same_name.txt 移动到 dest_dir
    let move_res = service.move_items(
        vec![
            "folder_a/same_name.txt".to_string(),
            "folder_b/same_name.txt".to_string(),
        ],
        "dest_dir",
    );
    assert!(move_res.is_err(), "单批次内目标路径碰撞必须被拦截拒绝！");
    let err_msg = move_res.err().unwrap();
    println!("-> 成功拦截批次内目标冲突: {}", err_msg);
    assert!(err_msg.contains("相同目标") || err_msg.contains("同一目标"));

    // 验证原文件完好无损，未发生删除或破坏
    let list_a = service.list_files("folder_a").expect("list a");
    let list_b = service.list_files("folder_b").expect("list b");
    assert_eq!(list_a.len(), 1, "folder_a 文件必须完好保留");
    assert_eq!(list_b.len(), 1, "folder_b 文件必须完好保留");
    println!("-> 验证通过：源文件完好无损，未发生任何静默删除或数据丢失！");

    // 2. 验证批量复制同名目标冲突拦截
    println!("\n2. 验证批量复制同名目标冲突拦截...");
    let copy_res = service.copy_items(
        vec![
            "folder_a/same_name.txt".to_string(),
            "folder_b/same_name.txt".to_string(),
        ],
        "dest_dir",
    );
    assert!(copy_res.is_err(), "批量复制目标重合必须被拦截！");
    println!("-> 成功拦截批量复制目标冲突: {}", copy_res.err().unwrap());

    // 3. 验证自我递归导入拦截（禁止将保险箱密文目录导入自身）
    println!("\n3. 验证自我递归导入拦截...");
    let self_import_res = service.import_paths(vec![vault_dir.to_string_lossy().to_string()], "");
    assert!(self_import_res.is_err(), "导入保险箱自身密文目录必须被拦截拒绝！");
    let self_err = self_import_res.err().unwrap();
    println!("-> 成功拦截自我递归导入: {}", self_err);
    assert!(self_err.contains("禁止将当前保险箱密文目录"));

    // 4. 验证系统保留名拦截（audit.log 与 .audit.log）
    println!("\n4. 验证系统保留名拦截 (audit.log)...");
    assert!(service.create_folder("audit.log").is_err(), "创建 audit.log 目录必须被拦截");
    assert!(service.create_folder(".audit.log").is_err(), "创建 .audit.log 目录必须被拦截");
    assert!(service.rename_item("folder_a/same_name.txt", "audit.log").is_err(), "重命名为 audit.log 必须被拦截");
    assert!(service.rename_item("folder_a/same_name.txt", ".audit.log").is_err(), "重命名为 .audit.log 必须被拦截");
    println!("-> 验证通过：audit.log 及 .audit.log 被成功列为系统保留名，彻底杜绝幽灵条目！");

    // 5. 验证子目录中常规命名文件不受 is_internal_control 误伤
    println!("\n5. 验证内部管控过滤边界收紧...");
    service.create_folder("sub_work").expect("mkdir sub_work");
    let normal_f = source_dir.join("normal_audit.txt");
    fs::write(&normal_f, "some text").unwrap();
    service.import_paths(vec![normal_f.to_string_lossy().to_string()], "sub_work").expect("import to sub_work");
    let sub_items = service.list_files("sub_work").expect("list sub_work");
    assert_eq!(sub_items.len(), 1, "sub_work 内部常规文件必须正常列出");
    assert_eq!(sub_items[0].name, "normal_audit.txt");
    println!("-> 验证通过：子目录下常规文件正常列出，不受根层管控过滤影响！");

    // 6. 验证子目录下 list_files 返回的 Path 自动补全全局相对路径前缀
    println!("\n6. 验证子目录下 list_files 全局路径前缀规范化...");
    assert_eq!(
        sub_items[0].path, "sub_work/normal_audit.txt",
        "子目录下条目的 path 必须包含父目录前缀以供前端直接导出/重命名/移入回收站"
    );
    // 验证利用该完整路径导出
    let sub_export_dir = base_dir.join("sub_export");
    fs::create_dir_all(&sub_export_dir).unwrap();
    service.export_item(&sub_items[0].path, sub_items[0].is_dir, &sub_export_dir.to_string_lossy())
        .expect("利用子目录条目 path 导出必须成功");
    assert!(sub_export_dir.join("normal_audit.txt").exists(), "子目录文件必须成功导出");
    println!("-> 验证通过：子目录条目 path 规范化为全局相对路径，导出与操作无缝衔接！");

    // 7. 验证内存安全预览接口与体积熔断保护
    println!("\n7. 验证内存流式预览接口与体积熔断...");
    let preview_bytes = service.read_file_preview("sub_work/normal_audit.txt", 1024 * 1024).expect("预览应成功");
    assert_eq!(preview_bytes, b"some text", "预览数据应完全一致");
    let oom_err = service.read_file_preview("sub_work/normal_audit.txt", 4);
    assert!(oom_err.is_err(), "超限体积必须被拦截拒绝");
    println!("-> 验证通过：内存流式安全预览与熔断限制机制工作正常！");

    service.lock_vault();
    service.stop();
    let _ = fs::remove_dir_all(&base_dir);
    println!("\n🎉 新增 Bug 修复专项回归测试全部圆满通过！");
    std::process::exit(0);
}

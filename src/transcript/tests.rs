use proptest::prelude::*;

use super::*;

fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
    Segment {
        start_ms,
        end_ms,
        text: text.to_string(),
    }
}

fn date() -> LocalTime {
    LocalTime {
        year: 2026,
        month: 10,
        day: 19,
        hour: 14,
        minute: 30,
        second: 5,
        offset_minutes: -180,
    }
}

fn sample() -> Transcript {
    Transcript::new(
        Header {
            file: "Reunião de orçamento.mp4".to_string(),
            date: date(),
            duration_ms: 3_725_400,
        },
        &[
            seg(0, 2_000, " Bom dia."),
            seg(2_500, 5_000, " Vamos falar do *orçamento* de_2027."),
            seg(8_000, 10_000, " Depois da pausa, [anexo] <a> `x` \\ fim."),
            seg(3_661_000, 3_700_000, " Até amanhã."),
        ],
    )
}

#[test]
fn pausa_maior_que_1_5_s_quebra_o_paragrafo() {
    let p = paragraphs(&[
        seg(0, 1_000, "um"),
        seg(2_500, 3_000, "dois"),
        seg(4_501, 5_000, "três"),
        seg(5_000, 6_000, "  "),
        seg(6_100, 7_000, "quatro\n cinco"),
    ]);
    assert_eq!(
        p,
        vec![
            Paragraph {
                start_ms: 0,
                text: "um dois".into()
            },
            Paragraph {
                start_ms: 4_501,
                text: "três quatro cinco".into()
            },
        ]
    );
    assert!(paragraphs(&[]).is_empty());
    assert!(paragraphs(&[seg(0, 1, " ")]).is_empty());
}

#[test]
fn formato_de_tempo() {
    assert_eq!(timestamp(0), "00:00:00");
    assert_eq!(timestamp(59_999), "00:00:59");
    assert_eq!(timestamp(3_725_400), "01:02:05");
    assert_eq!(timestamp(360_000_000), "100:00:00");
}

#[test]
fn cabecalho() {
    let txt = render_txt(&sample());
    assert!(txt.starts_with(
        "Arquivo: Reunião de orçamento.mp4\r\nData: 19/10/2026 14:30\r\nDuração: 01:02:05\r\n"
    ));
}

#[test]
fn txt_usa_crlf() {
    let txt = render_txt(&sample());
    assert!(txt.ends_with("\r\n"));
    assert_eq!(txt.matches('\n').count(), txt.matches("\r\n").count());
    insta::assert_snapshot!(txt.replace("\r\n", "⏎\n"));
}

#[test]
fn md() {
    insta::assert_snapshot!(render_md(&sample()));
}

#[test]
fn parse_le_o_txt() {
    let parsed = parse(&render_txt(&sample()));
    assert_eq!(parsed.file.as_deref(), Some("Reunião de orçamento.mp4"));
    assert_eq!(parsed.date.as_deref(), Some("19/10/2026 14:30"));
    assert_eq!(parsed.duration.as_deref(), Some("01:02:05"));
    assert_eq!(parsed.blocks.len(), 3);
    assert_eq!(parsed.blocks[2].start_ms, Some(3_661_000));
}

#[test]
fn parse_mantem_texto_fora_do_formato() {
    let parsed =
        parse("\u{feff}Notas soltas\nsegunda linha\n\n[00:01:02] fala\n\n[1:2:3] não é tempo\n");
    assert_eq!(parsed.file, None);
    assert_eq!(
        parsed.blocks,
        vec![
            Block {
                start_ms: None,
                text: "Notas soltas\nsegunda linha".into()
            },
            Block {
                start_ms: Some(62_000),
                text: "fala".into()
            },
            Block {
                start_ms: None,
                text: "[1:2:3] não é tempo".into()
            },
        ]
    );
    assert_eq!(parse(""), Parsed::default());
}

#[test]
fn nome_limpo() {
    assert_eq!(clean_name("Reunião: 1/2"), "Reunião 1 2");
    assert_eq!(clean_name("  a<b>c?*  ..."), "a b c");
    assert_eq!(clean_name("..."), "Sem nome");
    assert_eq!(clean_name(""), "Sem nome");
    assert_eq!(clean_name("con"), "_con");
    assert_eq!(clean_name("LPT1.backup"), "_LPT1.backup");
    assert_eq!(clean_name("COM10"), "COM10");
    assert_eq!(clean_name(&"é".repeat(200)).chars().count(), 80);
    assert_eq!(
        folder_name(&date(), "Reunião de orçamento.mp4"),
        "2026-10-19 1430 - Reunião de orçamento"
    );
}

#[test]
fn pasta_repetida_ganha_sufixo() {
    let tmp = tempfile::tempdir().unwrap();
    let a = create_unique_dir(tmp.path(), "x").unwrap();
    let b = create_unique_dir(tmp.path(), "x").unwrap();
    let c = create_unique_dir(tmp.path(), "x").unwrap();
    assert_eq!(a, tmp.path().join("x"));
    assert_eq!(b, tmp.path().join("x (2)"));
    assert_eq!(c, tmp.path().join("x (3)"));
}

#[test]
fn grava_os_tres_arquivos() {
    let tmp = tempfile::tempdir().unwrap();
    let t = sample();
    let manifest = Manifest {
        id: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
        name: t.header.file.clone(),
        media: MediaKind::Video,
        size_bytes: 1_234,
        duration_ms: t.header.duration_ms,
        created: t.header.date.rfc3339(),
        backend: "cpu".into(),
        model: "base".into(),
        files: Files {
            txt: "ignorado".into(),
            md: "ignorado".into(),
        },
        ..Manifest::default()
    };
    let w = write(tmp.path(), &t, &manifest).unwrap();
    assert_eq!(
        w.dir,
        tmp.path().join("2026-10-19 1430 - Reunião de orçamento")
    );
    assert_eq!(std::fs::read_to_string(&w.txt).unwrap(), render_txt(&t));
    assert_eq!(std::fs::read_to_string(&w.md).unwrap(), render_md(&t));
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&w.manifest).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["media"], "video");
    assert_eq!(json["created"], "2026-10-19T14:30:05-03:00");
    assert_eq!(json["files"]["txt"], TXT_FILE);
    assert_eq!(json["files"]["md"], MD_FILE);
    let back: Manifest = serde_json::from_value(json).unwrap();
    assert_eq!(back.id, manifest.id);
    let names: Vec<_> = std::fs::read_dir(&w.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 3, "sobrou temporário: {names:?}");

    let again = write(tmp.path(), &t, &manifest).unwrap();
    assert!(
        again
            .dir
            .ends_with("2026-10-19 1430 - Reunião de orçamento (2)")
    );
}

#[test]
fn gravacao_atomica_substitui() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.txt");
    write_atomic(&path, b"um").unwrap();
    write_atomic(&path, b"dois").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"dois");
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
}

fn segments() -> impl Strategy<Value = Vec<Segment>> {
    prop::collection::vec((0u64..5_000, 0u64..4_000, "(\\PC|[\t\n\r]){0,30}"), 0..12).prop_map(
        |parts| {
            let mut t = 0;
            parts
                .into_iter()
                .map(|(gap, len, text)| {
                    let start = t + gap;
                    t = start + len;
                    Segment {
                        start_ms: start,
                        end_ms: t,
                        text,
                    }
                })
                .collect()
        },
    )
}

fn header() -> impl Strategy<Value = Header> {
    ("(\\PC|\t){0,40}", 0u64..400_000_000).prop_map(|(file, duration_ms)| Header {
        file,
        date: date(),
        duration_ms,
    })
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_FOR_TESTS
        && !name
            .chars()
            .any(|c| c.is_control() || r#"<>:"/\|?*"#.contains(c))
        && !name.ends_with([' ', '.'])
        && !name.starts_with(' ')
        && !RESERVED.iter().any(|r| {
            let stem = name.split('.').next().unwrap().trim_end().to_uppercase();
            stem == *r
        })
}

const MAX_NAME_FOR_TESTS: usize = 80;
const RESERVED: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

proptest! {
    #[test]
    fn render_e_parse_do_txt_voltam_iguais(header in header(), segments in segments()) {
        let t = Transcript::new(header, &segments);
        let parsed = parse(&render_txt(&t));
        prop_assert_eq!(parsed.file, Some(normalize(&t.header.file)));
        prop_assert_eq!(parsed.date, Some(t.header.date.display()));
        prop_assert_eq!(parsed.duration, Some(timestamp(t.header.duration_ms)));
        let expected: Vec<Block> = t
            .paragraphs
            .iter()
            .map(|p| Block { start_ms: Some(p.start_ms / 1_000 * 1_000), text: p.text.clone() })
            .collect();
        prop_assert_eq!(parsed.blocks, expected);
    }

    #[test]
    fn md_contem_cada_paragrafo(header in header(), segments in segments()) {
        let t = Transcript::new(header, &segments);
        let md = render_md(&t);
        for p in &t.paragraphs {
            let line = format!("**[{}]** {}", timestamp(p.start_ms), escape_md(&p.text));
            prop_assert!(md.lines().any(|l| l == line), "falta {line:?}");
            // Desfazer o escape devolve o texto.
            let mut unescaped = String::new();
            let escaped = escape_md(&p.text);
            let mut chars = escaped.chars();
            while let Some(c) = chars.next() {
                unescaped.push(if c == '\\' { chars.next().unwrap() } else { c });
            }
            prop_assert_eq!(&unescaped, &p.text);
        }
    }

    #[test]
    fn nome_limpo_nunca_e_invalido_nem_vazio(name in "\\PC{0,120}|[ .]{0,5}(con|NUL|com3|lpt9)[ .a-z]{0,5}") {
        let clean = clean_name(&name);
        prop_assert!(valid_name(&clean), "{name:?} → {clean:?}");
    }
}

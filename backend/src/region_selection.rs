use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionRecord {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) code: String,
    pub(crate) parent_code: String,
    pub(crate) name: String,
    pub(crate) name_pinyin: String,
    pub(crate) name_short: String,
    pub(crate) level: i32,
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) type_name: Option<String>,
    pub(crate) post_code: Option<String>,
    pub(crate) source_url: Option<String>,
}

#[derive(Default, Serialize)]
pub(crate) struct RegionPath<'a> {
    pub(crate) province: Option<&'a RegionRecord>,
    pub(crate) city: Option<&'a RegionRecord>,
    pub(crate) district: Option<&'a RegionRecord>,
    pub(crate) township: Option<&'a RegionRecord>,
    pub(crate) village: Option<&'a RegionRecord>,
}

enum NameMatch<'a> {
    Missing,
    Unique(&'a RegionRecord),
    Ambiguous,
}

fn normalized_name(name: &str) -> &str {
    let name = name.trim();
    match name {
        "内蒙古自治区" => return "内蒙古",
        "广西壮族自治区" => return "广西",
        "宁夏回族自治区" => return "宁夏",
        "新疆维吾尔自治区" => return "新疆",
        "西藏自治区" => return "西藏",
        _ => {}
    }
    for suffix in [
        "社区居民委员会",
        "社区居委会",
        "村民委员会",
        "居民委员会",
        "街道办事处",
        "村委会",
        "居委会",
        "特别行政区",
        "自治州",
        "自治县",
        "自治区",
        "地区",
        "街道",
        "社区",
        "省",
        "市",
        "区",
        "县",
        "盟",
        "镇",
        "乡",
        "村",
    ] {
        if let Some(mut stem) = name.strip_suffix(suffix).filter(|stem| !stem.is_empty()) {
            if matches!(
                suffix,
                "社区居民委员会"
                    | "社区居委会"
                    | "村民委员会"
                    | "居民委员会"
                    | "村委会"
                    | "居委会"
            ) {
                stem = stem
                    .strip_suffix("社区")
                    .or_else(|| stem.strip_suffix("村"))
                    .filter(|shorter| !shorter.is_empty())
                    .unwrap_or(stem);
            }
            if matches!(suffix, "自治州" | "自治县" | "自治区") {
                // Autonomous names can contain several ethnic groups. Only known
                // ethnonyms are removed, so ordinary place names stay intact.
                const ETHNIC_GROUPS: &[&str] = &[
                    "土家族",
                    "布依族",
                    "哈萨克",
                    "柯尔克孜",
                    "维吾尔",
                    "达斡尔族",
                    "鄂伦春族",
                    "鄂温克族",
                    "仫佬族",
                    "毛南族",
                    "朝鲜族",
                    "景颇族",
                    "傈僳族",
                    "拉祜族",
                    "纳西族",
                    "哈尼族",
                    "蒙古族",
                    "蒙古",
                    "藏族",
                    "羌族",
                    "彝族",
                    "苗族",
                    "侗族",
                    "黎族",
                    "壮族",
                    "回族",
                    "满族",
                    "瑶族",
                    "白族",
                    "傣族",
                    "佤族",
                    "怒族",
                    "锡伯族",
                    "撒拉族",
                    "保安族",
                    "东乡族",
                    "布朗族",
                    "普米族",
                    "独龙族",
                    "裕固族",
                    "塔吉克族",
                    "阿昌族",
                ];
                while let Some(shorter) = ETHNIC_GROUPS
                    .iter()
                    .find_map(|ethnic| stem.strip_suffix(ethnic))
                    .filter(|shorter| !shorter.is_empty())
                {
                    stem = shorter;
                }
            }
            return stem;
        }
    }
    name
}

fn match_name<'a>(records: &[&'a RegionRecord], name: &str) -> NameMatch<'a> {
    let name = name.trim();
    if name.is_empty() {
        return NameMatch::Missing;
    }
    let exact: Vec<_> = records
        .iter()
        .copied()
        .filter(|region| region.name == name)
        .collect();
    let matches = if exact.is_empty() {
        let normalized = normalized_name(name);
        records
            .iter()
            .copied()
            .filter(|region| {
                let region_name = normalized_name(&region.name);
                region_name == name || region_name == normalized || region.name == normalized
            })
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [] => NameMatch::Missing,
        [region] => NameMatch::Unique(region),
        _ => NameMatch::Ambiguous,
    }
}

fn is_municipality(province: &RegionRecord) -> bool {
    matches!(
        province.name.as_str(),
        "北京市" | "天津市" | "上海市" | "重庆市"
    )
}

fn is_group(city: &RegionRecord) -> bool {
    matches!(
        city.name.as_str(),
        "市辖区" | "县" | "省直辖县级行政区划" | "自治区直辖县级行政区划"
    )
}

/// Resolve address-provider names into one catalog path. Matching never crosses
/// a resolved parent, and ambiguous aliases leave that level unresolved.
pub(crate) fn resolve_regions<'a>(
    records: &'a [RegionRecord],
    province_name: &str,
    city_name: &str,
    district_name: &str,
) -> RegionPath<'a> {
    let provinces: Vec<_> = records.iter().filter(|region| region.level == 2).collect();
    let NameMatch::Unique(province) = match_name(&provinces, province_name) else {
        return RegionPath::default();
    };
    let mut path = RegionPath {
        province: Some(province),
        ..RegionPath::default()
    };
    let cities: Vec<_> = records
        .iter()
        .filter(|region| region.level == 3 && region.parent_code == province.code)
        .collect();
    match match_name(&cities, city_name) {
        NameMatch::Unique(city) => {
            path.city = Some(city);
            let districts: Vec<_> = records
                .iter()
                .filter(|region| region.level == 4 && region.parent_code == city.code)
                .collect();
            if let NameMatch::Unique(district) = match_name(&districts, district_name) {
                path.district = Some(district);
            }
            return path;
        }
        NameMatch::Ambiguous => return path,
        NameMatch::Missing => {}
    }

    // Providers commonly repeat a municipality in the city field, or place a
    // county-level city there. A unique county determines its real catalog parent.
    let groups: Vec<_> = cities
        .iter()
        .copied()
        .filter(|city| is_group(city))
        .collect();
    let districts: Vec<_> = records
        .iter()
        .filter(|region| {
            region.level == 4 && cities.iter().any(|city| city.code == region.parent_code)
        })
        .collect();
    let municipal_alias =
        is_municipality(province) && normalized_name(city_name) == normalized_name(&province.name);
    let city_county = match match_name(&districts, city_name) {
        NameMatch::Unique(district) => Some(district),
        NameMatch::Ambiguous => return path,
        NameMatch::Missing => None,
    };
    if !city_name.trim().is_empty() && !municipal_alias && city_county.is_none() {
        return path;
    }
    let district = match match_name(&districts, district_name) {
        NameMatch::Unique(district) => {
            if city_county.is_some_and(|county| county.code != district.code) {
                return path;
            }
            Some(district)
        }
        NameMatch::Ambiguous => return path,
        NameMatch::Missing => city_county,
    };
    if let Some(district) = district {
        path.city = cities
            .iter()
            .copied()
            .find(|city| city.code == district.parent_code);
        path.district = Some(district);
    } else if is_municipality(province) && groups.len() == 1 {
        path.city = groups.first().copied();
    }
    path
}

/// Some catalog cities administer streets directly, with no county level.
pub(crate) fn township_parent<'a>(
    records: &'a [RegionRecord],
    path: &RegionPath<'a>,
) -> Option<&'a RegionRecord> {
    path.district.or_else(|| {
        path.city.filter(|city| {
            !records
                .iter()
                .any(|region| region.level == 4 && region.parent_code == city.code)
        })
    })
}

/// Lower levels resolve within their selected district or direct-administering city.
pub(crate) fn resolve_local_regions<'a>(
    records: &'a [RegionRecord],
    mut path: RegionPath<'a>,
    township_name: &str,
    village_name: &str,
) -> RegionPath<'a> {
    let Some(parent) = township_parent(records, &path) else {
        return path;
    };
    let townships: Vec<_> = records
        .iter()
        .filter(|region| region.level == 5 && region.parent_code == parent.code)
        .collect();
    let NameMatch::Unique(township) = match_name(&townships, township_name) else {
        return path;
    };
    path.township = Some(township);
    let villages: Vec<_> = records
        .iter()
        .filter(|region| region.level == 6 && region.parent_code == township.code)
        .collect();
    if let NameMatch::Unique(village) = match_name(&villages, village_name) {
        path.village = Some(village);
    }
    path
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};

    use flate2::read::GzDecoder;

    use super::*;

    fn catalog() -> Vec<RegionRecord> {
        let reader = BufReader::new(GzDecoder::new(
            include_bytes!("../assets/regions-2023.copy.gz").as_slice(),
        ));
        reader
            .lines()
            .map(|line| line.expect("seed line"))
            .filter_map(|line| {
                let fields: Vec<_> = line.split('\t').collect();
                let level = fields[7].parse().expect("seed level");
                (level <= 4
                    || matches!(
                        fields[3],
                        "110108000000"
                            | "110108005000"
                            | "110108011000"
                            | "441900000000"
                            | "441900003000"
                    ))
                .then(|| RegionRecord {
                    id: fields[1].into(),
                    version: fields[0].into(),
                    code: fields[2].into(),
                    parent_code: fields[3].into(),
                    name: fields[4].into(),
                    name_pinyin: fields[5].into(),
                    name_short: fields[6].into(),
                    level,
                    kind: None,
                    type_name: None,
                    post_code: None,
                    source_url: None,
                })
            })
            .collect()
    }

    fn names(path: RegionPath<'_>) -> (Option<&str>, Option<&str>, Option<&str>) {
        (
            path.province.map(|r| r.name.as_str()),
            path.city.map(|r| r.name.as_str()),
            path.district.map(|r| r.name.as_str()),
        )
    }

    #[test]
    fn municipalities_resolve_the_catalog_group_and_district() {
        let records = catalog();
        assert_eq!(
            names(resolve_regions(&records, "北京", "北京市", "朝阳")),
            (Some("北京市"), Some("市辖区"), Some("朝阳区"))
        );
        assert_eq!(
            names(resolve_regions(&records, "重庆市", "重庆", "奉节县")),
            (Some("重庆市"), Some("县"), Some("奉节县"))
        );
        assert_eq!(
            names(resolve_regions(&records, "北京市", "海淀区", "中关村街道")),
            (Some("北京市"), Some("市辖区"), Some("海淀区"))
        );
    }

    #[test]
    fn autonomous_names_and_directly_administered_counties_resolve() {
        let records = catalog();
        assert_eq!(
            names(resolve_regions(&records, "广西", "南宁", "青秀")),
            (Some("广西壮族自治区"), Some("南宁市"), Some("青秀区"))
        );
        assert_eq!(
            names(resolve_regions(&records, "新疆", "石河子市", "")),
            (
                Some("新疆维吾尔自治区"),
                Some("自治区直辖县级行政区划"),
                Some("石河子市")
            )
        );
        assert_eq!(
            names(resolve_regions(&records, "海南省", "文昌市", "龙楼镇")),
            (Some("海南省"), Some("省直辖县级行政区划"), Some("文昌市"))
        );
        assert_eq!(normalized_name("湘西土家族苗族自治州"), "湘西");
        assert_eq!(normalized_name("香港特别行政区"), "香港");
    }

    #[test]
    fn district_names_only_match_under_the_resolved_parent() {
        let records = catalog();
        assert_eq!(
            names(resolve_regions(&records, "吉林省", "长春市", "朝阳区")),
            (Some("吉林省"), Some("长春市"), Some("朝阳区"))
        );
        assert_eq!(
            names(resolve_regions(&records, "河北省", "石家庄市", "海淀区")),
            (Some("河北省"), Some("石家庄市"), None)
        );
        assert_eq!(
            names(resolve_regions(&records, "未知省", "北京市", "海淀区")),
            (None, None, None)
        );
        assert_eq!(
            names(resolve_regions(&records, "河南省", "未知市", "济源市")),
            (Some("河南省"), None, None)
        );
    }

    #[test]
    fn county_addresses_infer_only_their_unique_catalog_parent() {
        let records = catalog();
        assert_eq!(
            names(resolve_regions(&records, "江苏省", "昆山市", "")),
            (Some("江苏省"), Some("苏州市"), Some("昆山市"))
        );
        assert_eq!(
            names(resolve_regions(&records, "浙江省", "", "德清县")),
            (Some("浙江省"), Some("湖州市"), Some("德清县"))
        );
        assert_eq!(
            names(resolve_regions(&records, "江苏省", "昆山市", "常熟市")),
            (Some("江苏省"), None, None)
        );
        assert_eq!(
            names(resolve_regions(&records, "河北省", "", "新华区")),
            (Some("河北省"), None, None)
        );
        assert_eq!(
            names(resolve_regions(&records, "浙江省", "", "武康街道")),
            (Some("浙江省"), None, None)
        );
    }

    #[test]
    fn streets_and_communities_resolve_only_beneath_selected_parents() {
        let records = catalog();
        let upper = || resolve_regions(&records, "北京市", "北京市", "海淀区");
        let path = resolve_local_regions(&records, upper(), "中关村", "科源社区");
        assert_eq!(path.township.map(|r| r.name.as_str()), Some("中关村街道"));
        assert_eq!(
            path.village.map(|r| r.name.as_str()),
            Some("科源社区居委会")
        );
        let path = resolve_local_regions(&records, upper(), "中关村街道", "红民村");
        assert_eq!(
            path.village.map(|r| r.name.as_str()),
            Some("红民村社区居委会")
        );
        let path = resolve_local_regions(&records, upper(), "中关村街道", "核二院社区");
        assert!(path.township.is_some());
        assert!(path.village.is_none());
        let path = resolve_local_regions(&records, upper(), "", "科源社区");
        assert!(path.township.is_none());
        assert!(path.village.is_none());
    }

    #[test]
    fn partial_and_conflicting_group_addresses_do_not_guess() {
        let records = catalog();
        assert_eq!(
            names(resolve_regions(&records, "重庆市", "重庆市", "未知区")),
            (Some("重庆市"), None, None)
        );
        assert_eq!(
            names(resolve_regions(&records, "海南省", "文昌市", "琼海市")),
            (Some("海南省"), None, None)
        );
        assert_eq!(
            names(resolve_regions(&records, "江苏省", "", "")),
            (Some("江苏省"), None, None)
        );
    }

    #[test]
    fn cities_without_counties_use_their_direct_townships() {
        let records = catalog();
        let path = resolve_local_regions(
            &records,
            resolve_regions(&records, "广东省", "东莞市", ""),
            "东城",
            "岗贝社区",
        );
        assert_eq!(path.city.map(|r| r.name.as_str()), Some("东莞市"));
        assert!(path.district.is_none());
        assert_eq!(path.township.map(|r| r.name.as_str()), Some("东城街道"));
        assert_eq!(
            path.village.map(|r| r.name.as_str()),
            Some("岗贝社区居民委员会")
        );
        let path = resolve_local_regions(&records, RegionPath::default(), "中关村", "科源社区");
        assert!(path.township.is_none());
        assert!(path.village.is_none());
    }

    #[test]
    fn ambiguous_committee_aliases_are_rejected() {
        let mut records: Vec<_> = catalog()
            .into_iter()
            .filter(|region| region.level == 6)
            .take(2)
            .collect();
        records[0].name = "榴花社区居民委员会".into();
        records[1].name = "榴花村民委员会".into();
        let candidates: Vec<_> = records.iter().collect();
        assert!(matches!(
            match_name(&candidates, "榴花"),
            NameMatch::Ambiguous
        ));
        assert!(matches!(
            match_name(&candidates, "榴花社区居民委员会"),
            NameMatch::Unique(_)
        ));
        assert_eq!(
            normalized_name("唐洪村村民委员会"),
            normalized_name("唐洪村")
        );
    }

    #[test]
    fn exact_names_take_precedence_and_ambiguous_short_names_are_rejected() {
        let records = catalog();
        let mut same_names: Vec<_> = records
            .into_iter()
            .filter(|r| r.name == "朝阳区" || r.name == "朝阳县")
            .collect();
        assert!(same_names.len() > 1);
        let candidates: Vec<_> = same_names.iter().collect();
        assert!(matches!(
            match_name(&candidates, "朝阳"),
            NameMatch::Ambiguous
        ));
        same_names.retain(|r| r.name != "朝阳县");
        let county = RegionRecord {
            id: "test".into(),
            version: "test".into(),
            code: "test".into(),
            parent_code: "test".into(),
            name: "朝阳".into(),
            name_pinyin: String::new(),
            name_short: String::new(),
            level: 4,
            kind: None,
            type_name: None,
            post_code: None,
            source_url: None,
        };
        same_names.push(county);
        let candidates: Vec<_> = same_names.iter().collect();
        assert!(
            matches!(match_name(&candidates, "朝阳"), NameMatch::Unique(r) if r.code == "test")
        );
    }
}

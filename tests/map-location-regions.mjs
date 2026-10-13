import assert from 'node:assert/strict'
import test from 'node:test'
import { locationText, resolvedLocation } from '../frontend/src/lib/location.ts'

test('Chinese county is a district and is never promoted to city', () => {
  const result = resolvedLocation({ address: { state: '浙江省', county: '德清县', town: '武康街道' },
    display_name: '武康街道, 德清县, 湖州市, 浙江省, 中国' })
  assert.equal(result.province, '浙江省')
  assert.equal(result.city, '湖州市')
  assert.equal(result.district, '德清县')
  assert.equal(resolvedLocation({ address: { state: '浙江省', county: '德清县' } }).city, undefined)
})

test('county-level cities keep their district role when map supplies parent city in display name', () => {
  const result = resolvedLocation({ address: { state: '江苏省', county: '昆山市' },
    display_name: '玉山镇, 昆山市, 苏州市, 江苏省, 中国' })
  assert.equal(result.city, '苏州市')
  assert.equal(result.district, '昆山市')
})

test('a prefecture supplied as county uses the district from display name', () => {
  const result = resolvedLocation({ address: { state: '浙江省', county: '杭州市' },
    display_name: '文三路, 西湖区, 杭州市, 浙江省, 中国' })
  assert.equal(result.city, '杭州市')
  assert.equal(result.district, '西湖区')
})

test('a county-level city supplied as city uses the outer prefecture from display name', () => {
  for (const city of ['昆山市', '昆山']) {
    const result = resolvedLocation({ address: { state: '江苏省', city },
      display_name: '玉山镇, 昆山市, 苏州市, 江苏省, 中国' })
    assert.equal(result.city, '苏州市')
    assert.equal(result.district, city)
  }
})

test('municipalities provide both province and city candidates without losing county', () => {
  const result = resolvedLocation({ address: { state: '北京市', county: '东城区', road: '东长安街', house_number: '1号' } })
  assert.deepEqual(result, { province: '北京市', city: '北京市', district: '东城区', township: undefined,
    village: undefined, address: '东长安街 1号' })
  const cityOnly = resolvedLocation({ address: { city: '重庆市', city_district: '渝中区' } })
  assert.equal(cityOnly.province, '重庆市')
  assert.equal(cityOnly.city, '重庆市')
  assert.equal(cityOnly.district, '渝中区')
})

test('autonomous prefectures resolve as cities and autonomous regions remain provinces', () => {
  const result = resolvedLocation({ address: { state: '广西壮族自治区', city: '南宁市', county: '青秀区' } })
  assert.equal(result.province, '广西壮族自治区')
  assert.equal(result.city, '南宁市')
  assert.equal(result.district, '青秀区')
  const prefecture = resolvedLocation({ address: { state: '四川省', county: '阿坝藏族羌族自治州' } })
  assert.equal(prefecture.city, '阿坝藏族羌族自治州')
  assert.equal(prefecture.district, undefined)
})

test('search display names supply administrative candidates when address details are missing', () => {
  const result = resolvedLocation({ display_name: '人民公园，青秀区，南宁市，广西壮族自治区，中国' })
  assert.equal(result.province, '广西壮族自治区')
  assert.equal(result.city, '南宁市')
  assert.equal(result.district, '青秀区')
  assert.equal(result.address, '人民公园，青秀区，南宁市，广西壮族自治区，中国')
})

test('neighborhoods and streets are not district candidates', () => {
  const result = resolvedLocation({ address: { state: '广东省', city: '深圳市', suburb: '南山街道' },
    display_name: '南山街道, 南山社区, 深圳市, 广东省, 中国' })
  assert.equal(result.district, undefined)
})

test('IP estimates retain administrative candidates but suppress a precise address', () => {
  const result = resolvedLocation({ address: { state: '浙江省', city: '杭州市', city_district: '西湖区', town: '翠苑街道',
    neighbourhood: '翠苑一区社区', road: '文三路' } }, true)
  assert.equal(result.province, '浙江省')
  assert.equal(result.city, '杭州市')
  assert.equal(result.district, '西湖区')
  assert.equal(result.township, undefined)
  assert.equal(result.village, undefined)
  assert.equal(result.address, undefined)
})

test('map address details provide credible township and village candidates', () => {
  const result = resolvedLocation({ address: { state: '浙江省', city: '湖州市', county: '德清县', town: '莫干山镇',
    village: '庾村村', road: '黄郛西路', house_number: '8号' } })
  assert.deepEqual(result, { province: '浙江省', city: '湖州市', district: '德清县', township: '莫干山镇',
    village: '庾村村', address: '黄郛西路 8号' })
  assert.equal(locationText(result), '浙江省 湖州市 德清县 莫干山镇 庾村村 黄郛西路 8号')
})

test('township and village candidates fall back across provider keys and display names', () => {
  for (const townshipKey of ['town', 'township', 'subdistrict', 'suburb', 'municipality']) {
    const result = resolvedLocation({ address: { state: '广东省', city: '东莞市', [townshipKey]: '东城街道',
      quarter: '岗贝社区居民委员会' } })
    assert.equal(result.township, '东城街道')
    assert.equal(result.village, '岗贝社区居民委员会')
  }
  const displayed = resolvedLocation({ display_name: '太湖村村民委员会, 莫干山镇, 德清县, 湖州市, 浙江省, 中国' })
  assert.equal(displayed.township, '莫干山镇')
  assert.equal(displayed.village, '太湖村村民委员会')
  assert.equal(displayed.city, '湖州市')
  assert.equal(displayed.district, '德清县')
})

test('township municipalities are not interpreted as prefecture cities', () => {
  const result = resolvedLocation({ address: { state: '浙江省', municipality: '莫干山镇', county: '德清县' },
    display_name: '莫干山镇, 德清县, 湖州市, 浙江省, 中国' })
  assert.equal(result.city, '湖州市')
  assert.equal(result.township, '莫干山镇')
})

test('landmark and neighborhood names without village or township suffixes are not administrative candidates', () => {
  const result = resolvedLocation({ address: { state: '浙江省', city: '杭州市', town: '黄龙体育中心',
    suburb: '西湖景区', village: '人民公园', neighbourhood: '翠苑一区', quarter: 'XX工业园' } })
  assert.equal(result.township, undefined)
  assert.equal(result.village, undefined)
})

test('unavailable or malformed geocoder data is safe and does not invent administrative candidates', () => {
  for (const data of [null, undefined, 'unavailable', { error: 'Unable to geocode' }, { address: null }, { address: 'unavailable' }]) {
    const result = resolvedLocation(data)
    assert.equal(result.province, undefined)
    assert.equal(result.city, undefined)
    assert.equal(result.district, undefined)
    assert.equal(result.township, undefined)
    assert.equal(result.village, undefined)
  }
})

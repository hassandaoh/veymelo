# แผนงาน Veymelo

Veymelo เป็นโปรแกรมทำวิดีโอที่ **AI เป็นผู้ใช้หลัก** เขียนด้วย Rust ล้วน ไม่ใช้ไลบรารีภายนอก
(ใช้ของที่มากับระบบปฏิบัติการได้ เช่น AppKit, CoreGraphics, ImageIO, AVFoundation บน Mac)

ทุกส่วนออกแบบและเขียนเองทั้งหมด ผลลัพธ์ตรวจด้วยเทสต์และวาดภาพออกมาดู ไฟล์มาตรฐาน (OpenType, Unicode, glTF, SVG,
EXR, PSD, Lottie ฯลฯ) อ่านเขียนตามสเปกสาธารณะ ไลเซนส์: `LICENSE` (สงวนลิขสิทธิ์)

## หลักของโปรแกรม

- **ทุกการกระทำเป็นคำสั่งที่มีชื่อ** (`comp.new`, `layer.add`, `prop.set` …) มีคำอธิบายพารามิเตอร์
  และปฏิเสธพารามิเตอร์ที่ไม่รู้จัก ทุกช่องทางผ่านทะเบียนคำสั่งเดียวกัน
- **MCP server** ให้ AI ใช้ได้ทุกคำสั่ง ดูโปรเจกต์ และขอภาพเฟรมไปดูผลงาน
- **ทำงานได้ 2 แบบ**: ไม่มีหน้าต่าง (AI ทำเงียบๆ) และต่อกับแอปที่เปิดอยู่ (คนเห็นสดทุกการเปลี่ยนแปลง)
- **ไฟล์โปรเจกต์เป็น JSON** อ่านได้ทั้งคนและ AI มีเลขรุ่นของรูปแบบไฟล์
- **ทำเพื่อ AI**: งานใหม่ทุกชิ้นทำให้ AI ใช้ผ่านคำสั่ง/MCP และให้ AI เห็นผลผ่านภาพที่ขอดู (render_preview);
  หน้าตาสำหรับคน (ปุ่ม แผงตั้งค่า การลากแก้) **ทำเฉพาะเมื่อผู้ใช้สั่ง** — แอปมีไว้ให้คนดู AI ทำงาน
- เน้นคุณภาพ ไม่รีบ ทุกส่วนมีเทสต์ และวาดออกมาตรวจด้วยตา

## โครงสร้าง (workspace)

| ชั้น | crate | หน้าที่ |
|---|---|---|
| 0 | `json` | ค่า JSON อ่านและเขียนเอง |
| 0 | `time` | เวลาเป็นจำนวนเต็ม (flick = 1/705,600,000 วินาที) อัตราเฟรม timecode |
| 0 | `geom` | เวกเตอร์ เมทริกซ์ quaternion การแปลงตำแหน่งของชั้นภาพ |
| 0 | `color` | sRGB / linear, HSL / HSV, โหมดผสมสี |
| 0 | `zip` | deflate / inflate, zlib, CRC-32 และไฟล์ zip ของเราเอง (PNG และ `.lottie` ใช้ร่วมกัน) |
| 1 | `raster` | ภาพ float premultiplied การวาง/ผสมภาพ ตัวกรอง (เบลอบน GPU ได้) และภาพ 8 บิตของหน้าต่างแอป |
| 1 | `path` | เส้นทาง Bezier เติมสี เส้นขอบ และตัวแก้เส้นทาง |
| 1 | `font` | อ่านฟอนต์ (รวมฟอนต์ปรับน้ำหนักได้) จัดตัวอักษร |
| 1 | `keyframe` | ค่าที่เคลื่อนไหว keyframe และเส้นโค้งความเร็ว |
| 2 | `project` | เอกสารโปรเจกต์: comp ชั้นภาพ ต้นไม้ของค่า อ่านเขียน JSON |
| 2 | `effects` | ทะเบียนเอฟเฟกต์และตัวทำงาน |
| 3 | `render` | คำนวณและวาดเฟรม |
| 3 | `expr` | ภาษาสูตรควบคุมของเราเอง |
| 3 | `export` | ส่งออกวิดีโอ ภาพ GIF เสียง |
| 3 | `lottie` | Lottie (`.json` / `.lottie`) ส่งออกและนำเข้า |
| 1 | `os` | ส่วนที่คุยกับระบบปฏิบัติการ (Mac ก่อน): อ่านเขียนภาพ (ImageIO) วิดีโอและเสียง (AVFoundation) GPU (Metal) |
| 2 | `media` | ฟุตเทจ: บอกว่าไฟล์คืออะไร ลำดับภาพ และแคชเฟรมกับเสียงในหน่วยความจำ |
| 4 | `engine` | Session ทะเบียนคำสั่ง undo/redo |
| 5 | `mcp` | MCP server (ไม่มีหน้าต่าง / ต่อกับแอป) |
| 6 | apps `studio`, `cli` | แอปหน้าต่าง และคำสั่ง `veymelo` (mcp, render, exec) |

## ลำดับงาน

แต่ละขั้น: ออกแบบ → เขียน → เทสต์ + วาดตรวจ → คำสั่งให้ AI → ติ๊กเสร็จ

### ขั้น A: โครงพื้นฐาน (เสร็จ 2026-10-18 · เทสต์ 118 ผ่าน · clippy สะอาด)
- [x] แยกเป็น workspace ย้ายโค้ดเดิมเข้า crate (แอปหน้าต่างยังทำงานเหมือนเดิม)
- [x] `json`: อ่าน เขียน (สวยงาม) ทศนิยมกลับมาเท่าเดิม
- [x] `time`: tick อัตราเฟรม (รวม 29.97) การปัดเข้าเฟรม
- [x] `geom`: Vec2/Vec3 Mat3/Mat4 Quat การแปลงชั้นภาพ
- [x] `color`: sRGB / linear, HSL / HSV, hex, ชื่อโหมดผสมสี 38 แบบ (สูตรผสมอยู่ขั้น 1)
- [x] `keyframe`: ชนิดค่า, keyframe ครบทุกช่อง, linear / Bezier (ความเร็ว + อิทธิพล) / hold
      ตรงกับค่าที่วัดจากโปรแกรมจริง (ทางโค้งของตำแหน่ง, auto, roving อยู่ขั้น 2)
- [x] `project`: โครงเอกสาร (Project, Item, Comp, Layer, ต้นไม้ค่า) อ่านเขียนไฟล์ `.veymelo`
      นิยามค่าอยู่ในโค้ด ไฟล์เก็บเฉพาะที่ต่างจากค่าเริ่มต้น ไม่ใส่ id ก็ได้ (ตอนอ่านจะให้เอง)
      อ่านแบบเข้ม: ทุกข้อผิดบอกตำแหน่ง (`items[0].layers[2].props.transform.position`)
- [x] `engine`: Session ทะเบียนคำสั่ง ตรวจพารามิเตอร์ (ชนิดประกาศในโค้ด, "did you mean")
      undo/redo ประวัติแตกกิ่ง batch (`"$1.layer"`) คำสั่งทำบนสำเนา: พังแล้วไม่เปลี่ยนอะไร
- [x] `render`: เฟรมแรก (ชั้นสีพื้น ตำแหน่ง หมุน ย่อขยาย ความทึบ ชั้นแม่) และตัวเขียน PNG ของเราเอง
      (deflate + Huffman ของเราเอง) สิ่งที่ยังวาดไม่ได้บอกใน notes
- [x] `mcp`: MCP แบบไม่มีหน้าต่าง (command_list, command_describe, command_run, command_batch,
      doc_inspect, render_preview)
- [x] `cli`: `veymelo mcp`, `veymelo exec`, `veymelo render`, `veymelo commands`, `veymelo describe`
- [x] ต่อกับแอป: แอปเปิดช่องควบคุม (127.0.0.1 + รหัสลับในไฟล์ที่อ่านได้เฉพาะผู้ใช้) และ
      `veymelo mcp --bridge` (ทดสอบกับแอปจริงแล้ว: AI สร้าง comp และชั้นใน session ของแอป)

คำสั่งตอนนี้ (24): project.new/open/save/info, doc.inspect, edit.undo/redo, history.list/goto,
batch, view.set, command.list/describe, comp.new/set/info/remove, layer.add/remove/set/move,
prop.list/get/set/keys, render.frame

### ขั้น 1: ชั้นภาพ (เสร็จ 2026-10-18 · เทสต์รวม 150 ผ่าน)
- [x] ชนิดชั้น: สีพื้น, null, adjustment, comp ซ้อน (ตัวหนังสือ รูปทรง ฟุตเทจ กล้อง ไฟ อยู่ขั้นของมัน)
- [x] การแปลง: anchor, position, scale, rotation, opacity — วาดชั้นที่ขนาดที่เห็นจริง ขอบคมทุกขนาด
- [x] ชั้นลูกเกาะชั้นแม่ (parenting) + `keep` อยู่ที่เดิมบนจอเมื่อเปลี่ยนแม่
- [x] track matte (alpha, luma, กลับด้าน) — ตั้ง matte แล้วปิดภาพของชั้น matte ให้เอง
- [x] โหมดผสมสี 38 แบบ (สูตร W3C + แบบ classic, stencil, silhouette, dissolve, alpha add)
- [x] motion blur (มุมชัตเตอร์ เฟส จำนวนภาพปรับตามระยะที่เคลื่อน)
- [x] preserve transparency, collapse transformations, คุณภาพ best/draft/wireframe, bilinear/bicubic,
      solo, guide, 8/16/32 บิต (ปัดทุกขั้นเหมือนโปรเจกต์จริง), ผสมสีแบบ linear light
- [x] พื้นหลังของ comp ไม่ใช่ชั้น: โหมดผสมสีไม่ผสมกับมัน (วางไว้ใต้ภาพตอนจบ)
- [x] คำสั่งใหม่: layer.duplicate, layer.split, layer.precompose, project.set, ตั้งชัตเตอร์ใน comp.set
- [x] adjustment layer เปลี่ยนชั้นข้างล่างด้วยเอฟเฟกต์ของมัน — ทำแล้วในขั้น 7
- [x] motion blur ของชั้น comp ที่ collapse (การเคลื่อนของชั้นแม่) — ทำแล้วในขั้น 8

### ขั้น 2: การเคลื่อนไหว (เสร็จ 2026-10-18 · เทสต์รวม 164 ผ่าน)
- [x] keyframe: linear, Bezier (ความเร็ว + อิทธิพล), hold, auto, continuous, roving, Easy Ease
- [x] เส้นทางโค้งของตำแหน่ง (spatial Bezier): แทนเจนต์อัตโนมัติ (1/6 จากคีย์ข้างๆ) หรือกำหนดเอง
      เดินตามความยาวทาง ความเร็วคงที่บนทางโค้ง (วัดความยาวด้วย Gauss-Legendre + Newton)
- [x] ตำแหน่งแยกแกน (separate dimensions) ย้ายคีย์ไปมาได้
- [x] เวลา: time remap, freeze frame, ยืดย่อ (stretch), เล่นย้อนทั้งชั้น, ย้อนคีย์, work area,
      marker ของ comp และของชั้น (ย้ายตามชั้น), ตรงเฟรมทุกอัตรา (tick)
- [x] คำสั่ง: key.list/set/ease/move/remove/reverse, layer.timeRemap/freeze/reverse/stretch,
      marker.add/list/remove

### ขั้น 3: รูปทรงและหน้ากาก (เสร็จ 2026-10-18 · เทสต์รวม 201 ผ่าน)
- [x] รูปทรง: สี่เหลี่ยม (มุมมน) วงรี ดาว รูปหลายเหลี่ยม เส้นทาง เติมสี (non-zero / even-odd)
      เส้นขอบ (cap, join, miter, ขีดประ 3 คู่) ไล่สีแบบเส้นตรงและวงกลม (มีจุดสว่าง) กลุ่มที่มี transform ของตัวเอง
- [x] ตัวแก้รูปทรง: trim paths (พร้อมกัน/ทีละเส้น), repeater, round corners, offset, zig zag, twist,
      wiggle, merge paths (รวม ลบ ตัดกัน ไม่ทับกัน — บูลีนของเราเอง), pucker & bloat
- [x] ลำดับแบบโปรแกรมจริง: สีทาเส้นด้านบน ตัวแก้มีผลกับทุกเส้นในกลุ่มแม้อยู่ใต้สี, composite above/below
- [x] หน้ากาก: โหมด 6 แบบ กลับด้าน ขอบนุ่ม (แยกแนวนอน/ตั้ง) ขยาย/หด ความทึบ
- [x] ตัวเติมสีใหม่ของเรา: นับรอบตามกฎ 16 แถวย่อยต่อพิกเซล แนวนอนตรงทุกเศษ
- [x] คำสั่ง: layer.add shape, shape.add/remove, mask.add/remove (สี่เหลี่ยม วงรี เส้นทาง)
- [x] taper และ wave ของเส้นขอบ, จุดขนนกของหน้ากาก, motion blur ของหน้ากาก [เสร็จในขั้น 11]

### ขั้น 4: ตัวหนังสือ (เสร็จ 2026-10-18 · เทสต์รวม 217 ผ่าน)
- [x] จัดตัวอักษรจริงด้วยตัวจัดของเราเอง: GSUB (แทนที่ 1–7: เดี่ยว หลายตัว ทางเลือก ลิเกเจอร์ บริบท
      บริบทต่อเนื่อง ส่วนขยาย) GPOS (ระยะคู่ วางเครื่องหมายบนฐาน บนลิเกเจอร์ บนเครื่องหมาย บริบท)
      kern เก่า ภาษาไทย: แยกสระอำ ย้ายนิคหิต วางสระ/วรรณยุกต์ — ตรวจตรงกับ CoreText ทุกตำแหน่ง
- [x] หาฟอนต์ในเครื่อง (ชื่อ family/full/PostScript น้ำหนัก เอียง .ttc) ฟอนต์สำรองตามภาษา (ไทย → ThonburiUI)
      ฟอนต์ปรับน้ำหนักได้ตั้งน้ำหนักตรงค่า
- [x] ตัดคำไทยด้วยพจนานุกรมของระบบ (CFStringTokenizer) ตัดบรรทัดที่ช่องว่าง ขีด อักษรจีน/ญี่ปุ่น
- [x] ข้อความบรรทัดเดียว (point) และย่อหน้าในกรอบ (box) ชิดซ้าย/กลาง/ขวา/เต็มบรรทัด 4 แบบ
      tracking, leading, baseline shift, ตัวใหญ่ทั้งหมด/ตัวใหญ่เล็ก, เส้นขอบบน/ใต้สีเติม
- [x] text animator: ตำแหน่ง anchor ขนาด เอียง หมุน ความทึบ สีเติม สีเส้น ความหนาเส้น tracking
      จุดหมุนตามตัว/คำ/บรรทัด/ทั้งหมด
- [x] range selector (เปอร์เซ็นต์/ลำดับ, ตัวอักษร/ไม่รวมช่องว่าง/คำ/บรรทัด, 6 โหมด, 6 รูปทรง, สุ่มลำดับ)
      wiggly selector
- [x] คำสั่ง: layer.add text, text.set, text.animator.add, text.selector.add
- [x] ตัวหนังสือบนเส้นทาง, หลายสไตล์ในข้อความเดียว, ข้อความแนวตั้ง, ภาษาเขียนขวาไปซ้าย,
      ฟอนต์ CFF (.otf) และฟอนต์ AAT ล้วน (Ayuthaya ฯลฯ), ease high/low ของ selector [เสร็จในขั้น 11]

### ขั้น 5: สูตรควบคุม (expressions) (เสร็จ 2026-10-18 · เทสต์รวม 232 ผ่าน)
- [x] ภาษาของเราเอง (crate `expr`): เขียนแบบ expression ของโปรแกรมคอมโพสิต — ตัวแปร ฟังก์ชัน if/for/while
      อาร์เรย์บวกลบคูณหารทีละส่วน สตริง `push` `map` `filter` `reduce` · ตัวอ่าน+ตัวรันของเราเอง จำกัดจำนวนก้าว
- [x] ชื่อ: time value thisComp thisLayer thisProperty index numKeys velocity speed, ชื่อของชั้นเรียกตรงได้
      (position, transform, effect(), mask(), content(), text), Math ครบ
- [x] วัตถุ: composition (layer(ชื่อ/ลำดับ/ชั้น,ห่าง) width height duration frameDuration marker…),
      ชั้น (name index inPoint outPoint parent source toComp fromComp…), กลุ่ม, property (valueAtTime
      velocityAtTime key(n) nearestKey numKeys), key, marker (key ตามลำดับหรือข้อความ nearestKey)
- [x] ฟังก์ชัน: wiggle temporalWiggle smooth loopOut/loopIn (cycle pingpong offset continue + Duration)
      linear ease easeIn easeOut clamp random gaussRandom seedRandom noise add sub mul div dot cross length
      normalize lookAt timeToFrames framesToTime timeToTimecode posterizeTime rgbToHsl hslToRgb hexToRgb comp()
- [x] สุ่มและ wiggle เหมือนเดิมทุกครั้ง: seed จาก id ชั้น + path ของ property (คัดลอก/บันทึกแล้วยังเหมือนเดิม)
- [x] ชั้น 2D เห็นจุดเป็น 2 ส่วน [x, y]; ผลเกินขอบเขตถูกจับไว้ที่ขอบ (ความทึบ 130 → 100); ข้อความใหม่คงสไตล์เดิม
- [x] สูตรที่วนกลับหาตัวเอง หรือพัง: ใช้ค่าจาก key แทน แล้วบอกเหตุผลใน notes (บอกตำแหน่งที่เขียนผิด)
- [x] renderer อ่านทุกค่าผ่านสูตร: transform พ่อแม่ ความทึบ time remap shape mask text animator selector
- [x] คำสั่ง: prop.expression (ใส่/ลบ/ปิด), prop.evaluate (ลองสูตรหลายเวลาโดยไม่เก็บ), prop.get/list แสดง
      value หลังสูตร + keyed + error
- [x] sourceRectAtTime, sampleImage, propertyGroup(n), สูตรที่ให้ path (createPath), audioLevels,
      expression selector ของ text animator [เสร็จในขั้น 11: ภาพที่สูตรอ่านมาจากตัววาดผ่าน trait Pictures
      (กันวนลึกเกินสองชั้น), propertyIndex ด้วย; audioLevels อ่านเป็นค่า audio.audioLevels]

### ขั้น 6: หุ่นเชิด (puppet) (เสร็จ 2026-10-18 · เทสต์รวม 246 ผ่าน)
หุ่นเชิดในโปรแกรมต้นแบบเป็นเอฟเฟกต์ จึงทำโครงเอฟเฟกต์ก่อน (ขั้น 7 จะเติมเอฟเฟกต์อื่นลงโครงนี้)
- [x] โครงเอฟเฟกต์ (crate `effects`): เอฟเฟกต์แต่ละตัวเป็นชนิดในรายการ `effects` ของชั้น ค่าของมันเป็น
      property ธรรมดา (keyframe/สูตร/path ได้) ทำงานในพื้นที่ของชั้น (`Buf`: ภาพ + จุดเริ่ม + พิกเซลต่อหน่วย)
      บนลงล่าง หลังหน้ากาก ก่อนวางลง comp ชั้นที่มีเอฟเฟกต์วาดเต็มชั้น ไม่ตัดเฉพาะส่วนที่เห็น
- [x] mesh: alpha ครึ่งหนึ่งขึ้นไปบนตารางหน่วยของชั้น (ไม่ขึ้นกับขนาดพรีวิว) ขยาย/หดด้วย distance
      transform จริง เลือกส่วนที่ต่อกับจุด seed อุดรู เดินรอบขอบเซลล์ ลดจุด แบ่งด้าน เติมจุดหกเหลี่ยม
      ใส่หมุดเป็นจุดยอดตรงตำแหน่ง Delaunay + กู้ขอบด้วยการพลิกขอบ เก็บ mesh ที่สร้างแล้ว (ภาพเดิม = mesh เดิม)
- [x] ดัดให้แข็งที่สุด (as-rigid-as-possible สองขั้นของ Igarashi) แก้สมการด้วย conjugate gradient ของเราเอง
- [x] หมุด 5 แบบ: ตำแหน่ง, ขั้นสูง (ตำแหน่ง หมุน ขยาย), ดัด (หมุน ขยาย), แข็ง (amount, extent), ซ้อน (inFront)
- [x] วาดผ่านสามเหลี่ยมหลังไปหน้า สีจากสามเหลี่ยมหน้าสุด ความทึบจากจุดสุ่ม 4 จุดรวมกัน: ไม่มีรอยต่อ
      ส่วนที่ไม่มี mesh ไม่ถูกวาด (เหมือนโปรแกรมต้นแบบ — เพิ่ม expansion ให้คลุม)
- [x] คำสั่ง: effect.add/remove/move/types, puppet.pin (หา/สร้าง Puppet และ mesh ที่คลุมจุดเอง),
      puppet.info, item.remove (เอาอะไรก็ได้ออกจากรายการ)
- [ ] mesh ที่จำรูปร่างตอนสร้างไว้ (ตอนนี้สร้างจากภาพเฟรมปัจจุบัน), เส้นตาข่ายบนภาพพรีวิวให้ AI ดู,
      motion blur ของ mesh, ปักหมุดตามเส้นทาง — ภายหลัง

### ขั้น 7: เอฟเฟกต์ (เสร็จ 2026-10-19 ยกเว้นที่รอตัวติดตาม/ตัวหาใบหน้า และเสียง · เทสต์รวม 551 ผ่าน)
- [x] พื้นฐาน: adjustment layer (เอฟเฟกต์ทำกับภาพข้างใต้ ตามพื้นที่ของชั้น หน้ากาก ความทึบ blend mode),
      เอฟเฟกต์อ่านชั้นอื่นได้ (ชั้นแผนที่ ยืดเต็มหรือวางกลาง กันวนกลับหาตัวเอง), จุดเริ่มที่กลางชั้น,
      ปัดเป็น 8/16 บิตหลังทุกเอฟเฟกต์, ตัวกรองภาพของเราเอง (Gaussian จริง + กล่องกว้างเป็นเศษได้: เบลอโตอย่างลื่น)
- [x] Expression Controls (8): slider angle checkbox color point 3D point layer dropdown
- [x] Blur & Sharpen (15): Gaussian, Fast Box, Directional, Radial, Sharpen, Unsharp Mask, Channel, Compound,
      Bilateral, Smart, Cross, Radial Fast, Vector, Camera Lens (ม่านรูรับแสง ขอบเรือง แสงจ้า แผนที่ระยะ),
      Reduce Interlace Flicker, Zoom Blur 5 แบบ (straight/fading/centered/rotate/rotate fading), Camera-Shake Deblur
      (แบ่งเป็นแผ่น เทียบความคมกับเฟรมข้างๆ แผ่นที่คมกว่าเลื่อนให้ตรงแล้วแทนที่ ผสมรอยต่อ) [เสร็จ]
- [x] Color Correction (35 จาก 38): Tint, Tritone, Brightness & Contrast (โค้งใหม่ + แบบเก่า), Hue/Saturation (6 ช่วงสี
      + colorize), Levels (ทุกช่อง, clip ตาม 8/16/32 บิต), Levels (Individual Controls), Exposure (linear), Black & White,
      Change to Color, Change Color, Leave Color, Photo Filter (21 ฟิลเตอร์), Vibrance, Color Balance, Color Balance (HLS),
      Channel Mixer, Gamma/Pedestal/Gain, Colorama (วงจานสีของเอง/gradient, add phase, mask), Curves (โค้งไม่ล้น),
      Auto Levels/Contrast/Color (+ เฉลี่ยข้ามเฟรม), Equalize, Shadow/Highlight, Selective Color, Color Link,
      Broadcast Colors, Video Limiter, Toner, Color Offset, Kernel, Color Neutralizer, PS Arbitrary Map,
      Color Grade (ตัวเกรดเต็ม: LUT, ไวต์บาลานซ์, โทน, look, curves, วงล้อสี, secondary, vignette), Apply Color LUT (.cube ของเรา)
      — OCIO 5 ตัว + Color Stabilizer ทำแล้ว (ดูหมวดจัดการสีด้านล่าง)
- [x] Generate และการวาด (24): Gradient Ramp, 4-Color Gradient, Checkerboard, Grid, Circle, Ellipse, Beam,
      Lens Flare (แสงสะท้อนในเลนส์ของเราเอง), Cell Pattern, Light Rays, Light Burst, Light Sweep,
      Radio Waves (วงกลม หลายเหลี่ยม ดาว ขอบภาพของชั้นอื่น หน้ากาก, ค่าตอนเกิดหรือทุกเฟรม, เด้งขอบ, 7 โปรไฟล์เส้น),
      Advanced Lightning (8 แบบ, แตกกิ่ง, หลบหรือเข้าหาส่วนทึบ, แกนกลางจางตามระยะ), Fractal (Mandelbrot/Julia,
      inverse, over Julia, ยกกำลัง 2–4, 4 จานสี, oversample เฉพาะขอบ), Fill, Stroke, Scribble, Vegas, Write-on,
      Paint Bucket, Eyedropper Fill, Glue Gun, Threads — Audio Spectrum/Waveform รอขั้น 10
- [x] Distort (36 จาก 38): Twirl, Spherize, Bulge, Wave Warp (9 คลื่น ตรึงขอบ), Ripple, Mirror, Offset, Polar Coordinates,
      Transform (ขยายบัฟเฟอร์ได้ + motion blur ของตัวเอง/ตามคอมพ์), Corner Pin, Turbulent Displace (noise เรียบของเราเอง วนรอบได้),
      Displacement Map (กลาง/ยืด/ปูกระเบื้อง), Optics Compensation, Magnify, Mesh Warp (โค้งเรียบด้วย spline), Bezier Warp,
      Bend, Lens, Page Turn, Tiler, Griddler, Bender, Blobbylize, Power Pin, Ripple Pulse (ตามระดับที่ animate),
      Slant, Smudge, Split, Split 2, Smear, Reshape (จุดคู่ได้), Twirl (Legacy), Warp (15 แบบ), Detail-preserving Upscale,
      Flow Motion, Liquify (12 พู่กัน, reconstruction 4 แบบ, freeze mask, คำสั่ง liquify.stroke)
      — Rolling Shutter Repair และ Warp Stabilizer ทำแล้วในขั้น 11 (ตัวติดตามการเคลื่อนไหว)
      — ภาพที่ถูกย้าย/ยืด (ชั้นและเอฟเฟกต์) ขอบคมตามพื้นที่ที่ทับจริง ไม่จางซ้ำจาก bilinear
- [x] Stylize (25): Glow (A&B, arbitrary map, 17 operation), Mosaic, Posterize, Threshold, Find Edges, Emboss, Color Emboss,
      Cartoon (guided filter ของเราเอง, shock filter), Roughen Edges (8 แบบ), Scatter, Strobe Light, Texturize, Brush Strokes,
      Kaleidoscope, Repeat Tile, Motion Tile, Vignette, Channel Threshold, Threshold RGB, Block Load, Burn Film, Glass, Plastic,
      Hex Tile, Smoothie
- [x] Noise & Grain (13): Fractal Noise (9 แบบ, 4 noise type, sub settings, evolution วนรอบได้พอดี, perspective offset,
      24 blending mode), Turbulent Noise, Curl Noise, Noise, Noise Alpha, Noise HLS, Noise HLS Auto, Add Grain (preset, preview),
      Match Grain (วัดเกรนจากกล่องที่เรียบที่สุด), Remove Grain (guided filter + unsharp + temporal), Median (histogram เลื่อน),
      Median (Legacy), Dust & Scratches
- [x] Channel (13): Invert (RGB/HLS/YIQ/alpha), Set Channels, Shift Channels, Channel Combiner, Minimax (คิวเดียวต่อแถว ไม่ช้าตามรัศมี),
      Arithmetic, Solid Composite, Remove Color Matting, Set Matte, Blend, Calculations, Composite (ภาพก่อนเอฟเฟกต์), Compound Arithmetic
- [x] Matte: Simple Choker (โตหรือหดแบบกลม), Matte Choker, Refine Soft Matte, Refine Hard Matte (guided filter, ลดสั่นข้ามเฟรม,
      motion blur, ล้างสีพื้นหลังที่ขอบ) — ที่ยังไม่ทำ: Object Matte (ต้องมีตัวแยกวัตถุและตัวติดตามก่อน)
- [x] Keying (11 + เก่า 3): Screen Keyer (ตัวคีย์จอเขียว/น้ำเงินเต็มรูปแบบ: bias, clip + rollback, despot, หน้ากากในและนอก,
      แก้สีตัวและขอบ, crop), Linear Color Key, Color Difference Key, Color Range, Extract, Difference Matte, Inner/Outer Key
      (หน้ากาก 11 คู่ + พู่กันล้าง 20), Unmult, Advanced Spill Suppressor, Key Cleaner, Wire Removal; Color Key, Luma Key,
      Spill Suppressor (หมวด obsolete)
- [x] Perspective (9 จาก 10): Drop Shadow, Radial Shadow, Bevel Alpha, Bevel Edges, Cylinder, Sphere (ขอบเรียบ), Spotlight,
      Environment, 3D Glasses — ที่ยังไม่ทำ: 3D Camera Tracker (ต้องมีตัวติดตามก่อน)
- [x] Transition (17): Block Dissolve, Gradient Wipe, Iris Wipe, Linear Wipe, Radial Wipe (ขอบนุ่มกว้างเท่ากันทุกระยะ),
      Venetian Blinds, Glass Wipe, Grid Wipe, Image Wipe, Jaws, Light Wipe, Line Sweep, Radial Scale Wipe, Scale Wipe, Twister,
      Warp Reveal, Card Wipe (ใช้ระบบการ์ด 3D เดียวกับ Card Dance)
- [x] Time (8): Echo (7 วิธีรวม), Posterize Time, Time Difference (เทียบตัวเองหรือเลเยอร์อื่น, alpha 9 แบบ), Time Displacement,
      Timewarp (Whole Frames / Frame Mix / Pixel Motion, เลือกด้วยความเร็วหรือเฟรม, motion blur, matte layer แยกตัวกับพื้นหลัง,
      warp layer, crop, ปรับละเอียด), Force Motion Blur, Wide Time, Pixel Motion Blur
      — Pixel Motion: หา motion แบบ block เป็นชั้น (pyramid) + บอกความมั่นใจแยกแกน, smooth แบบให้น้ำหนักตามความมั่นใจ,
      ให้แต่ละพิกเซลเลือก motion ของตัวเองด้วย patch สี (น้ำหนักตามความเหมือน, ตัดค่าผิดเกิน), หาส่วนที่ถูกบัง/เพิ่งโผล่
      จากการที่ไม่มีอะไรตกลงมา แล้วใช้ motion จากเฟรมก่อน/หลังแทน, เฟรมกลางเลือกจากทั้งสองเฟรมเฉพาะที่มองเห็นจริง
      (ไม่มีภาพซ้อน) — 1080p ราว 2 วินาทีต่อเฟรม
- [x] Text (2): Numbers (10 แบบ: ตัวเลข, เติมศูนย์, timecode 30/25/24, เวลา, วันที่ 3 แบบ (วันนี้ตามเครื่องได้), ฐานสิบหก;
      สุ่มทุกเฟรม, เติม/เส้นขอบ 4 แบบ, ตัวเลขกว้างเท่ากัน), Timecode (SMPTE + drop-frame, เฟรม, ฟุต+เฟรม 35/16 มม.,
      เวลาของเลเยอร์/คอมพ์/กำหนดเอง, กล่อง) — วาดด้วยฟอนต์จริงผ่านเครื่องจัดตัวอักษรของเรา (เลือกฟอนต์ได้) ไม่ใช่ฟอนต์เส้น
- [x] Utility (7 จาก 9): Grow Bounds, Overbrights, HDR Highlight Compression, HDR Compander, Cineon Converter,
      Color Profile Converter (perceptual ดึงทั้งขอบเขตสีของต้นทางเข้าปลายทางพอดี), Apply Color LUT
      — ที่ยังไม่ทำ: Face Measurements, Face Track Points (ต้องมีตัวหาใบหน้าก่อน)
- [x] จัดการสี: ระบบปริภูมิสีใน veymelo-color (primaries + white + เส้นโค้ง: sRGB, BT.1886, PQ, HLG, ACEScct/cc, LogC3/4,
      S-Log3, V-Log, Canon Log 2/3, Log3G10, BMD Film Gen5, D-Log; Bradford; ACES gamut compression) ตรวจกับค่าที่ตีพิมพ์,
      ไฟล์สี (.cube .3dl .csp .spi1d .spi3d .spimtx .cc .ccc .cdl และ .clf ของ Academy) เดินหน้าและถอยหลังได้,
      ตัวอ่าน OCIO config (YAML ของเราเอง, v1/v2: roles, display spaces, view transforms, looks, search path,
      transform ทุกชนิดหลัก) + config ในตัวจากปริภูมิสีของเรา (Standard/Tone Mapped/Raw);
      เอฟเฟกต์: OCIO CDL / Color Space / Display / File / Look Transform, Color Stabilizer
      — ที่ยังไม่ทำ: ACES output transforms (RRT+ODT) ใน config ของ ACES (ผ่านไปพร้อมคำเตือน), GradingPrimary/Tone/Curve
- [x] Simulation (18): Rainfall, Snowfall, Bubbles, Drizzle, Star Burst (คำนวณจากเวลาตรงๆ ไม่พึ่งเฟรมก่อน);
      Particle World, Particle Systems, Mercury, Particle Playground (ปืน ตาราง ระเบิดเลเยอร์ ตัวอักษร map layer),
      Hair, Foam — จำลองทีละสเต็ป 60 ครั้งต่อวินาที เก็บสถานะทุกครึ่งวินาที (เลื่อนไปมาเร็ว, เฟรมเดียวกันได้ผลเดียวกัน
      ไม่ว่าเล่นมาหรือกระโดดมา); Card Dance, Shatter (ลายแก้ว จิ๊กซอว์ อิฐ วงซ้อน, แตกจากจุดแรกที่แรงโดน),
      Ball Action, Pixel Polly, Scatterize (ชิ้นส่วน 3D วาดด้วยตัววาดฉากของเราเอง);
      Caustics (หักเหตามกฎของสเนลล์ ผิวสูงตามขนาดเลเยอร์จึงดูเหมือนกันทุกขนาด, แสงรวมใต้ยอดคลื่นแบบส่งแสงลงไปจริง,
      ท้องฟ้าสะท้อน), Wave World (สมการคลื่นแบบอนุรักษ์พลังงาน, ขอบเปิดปล่อยคลื่นออกจริง, น้ำตื้นคลื่นช้าลง, ที่แห้งเป็นกำแพง,
      height map ต่อเนื่องแบบ cubic), Foam (ลม ปั่นป่วน flow map, ผลักกันและติดกันเป็นก้อน, แตกแล้วดันรอบข้าง,
      ฟองสบู่สีรุ้งจากฟิล์มบาง, สะท้อน environment map)
- [x] Immersive Video (12): VR Blur, VR Sharpen, VR De-Noise, VR Glow (เบลอบนทรงกลมจริง: ข้ามรอยต่อซ้ายขวา, แนวนอนกว้างขึ้นตาม
      1/cos ละติจูดแบบความกว้างเศษส่วนไม่เป็นขั้น, แนวตั้งเดินรอบวงกลมใหญ่ทั้งวงจึงข้ามขั้วได้จริง), VR Rotate Sphere, VR Converter
      (equirectangular, cube map 5 แบบ รวม equal-angle, sphere map, dome, ภาพแบน — สุ่มหลายจุดตามจำนวนพิกเซลต้นทางที่พิกเซลหนึ่งครอบ
      จึงไม่กระพริบ, หมุนด้วย cubic ที่ไม่เกิดขอบเงา), VR Sphere to Plane, VR Plane to Sphere, VR Chromatic Aberrations,
      VR Color Gradients, VR Fractal Noise (noise บนทรงกลม ไม่มีรอยต่อ ไม่บีบที่ขั้ว, evolution เปลี่ยนในที่โดยความแรงคงที่),
      VR Digital Glitch; ภาพสเตอริโอ (บน/ล่าง, ซ้าย/ขวา) ทำทีละตา
- [x] 3D Channel (8): 3D Channel Extract, Channel Extractor (แทน EXtractoR), Depth Matte, ID Matte, ID Viewer (แทน IDentifier),
      Cryptomatte (ตามสเปก: MurmurHash3 ตรวจกับค่าที่ตีพิมพ์, manifest, ชื่อ/ลาย */<hex>, ขอบตาม coverage ของ render),
      Fog 3D, Depth of Field (ของเราเอง: แบ่งความลึกเป็นชั้นห่างกันไม่เกิน 1.5 px ของเบลอ วาดไกลไปใกล้ ชั้นที่ถูกบังต่อสีของตัวเอง
      ใต้ของที่ใกล้กว่า — ของในโฟกัสคมจริง ของหน้าเบลอทับได้ ไม่มีขอบดำ); ขอบที่หยักใช้ 4 จุดต่อพิกเซล
      — ช่องข้อมูล (passes) มาพร้อมฟุตเทจ EXR และจากคอมพ์ซ้อน (ฉาก 3D ของเรา) แล้ว
- [x] Obsolete (9): Basic 3D (ย้อนแสงจากแต่ละพิกเซล สุ่มหลายจุดเมื่อเอียงมาก, ประกายสะท้อนแสงจากซ้ายบนหน้า, wireframe),
      Gaussian Blur (Legacy), Basic Text และ Path Text (ฟอนต์จริงผ่านเครื่องจัดตัวอักษรของเรา รวมฟอนต์สำรองตามภาษา;
      Path Text: Bezier/วงกลม/ห่วง (กระจายรอบวงพอดี)/เส้นตรง/หน้ากากของชั้น, kerning, เฉือน, ย่อขยาย, ขอบ, จาง, สั่นทุกเฟรม),
      Lightning (สายฟ้าแบบเก่า แตกกิ่ง ประกาย แรงดึง ผสมแบบ normal/add/screen); Color Key, Luma Key, Spill Suppressor,
      Reduce Interlace Flicker ทำไว้แล้วในหมวดของมัน
- เอฟเฟกต์ครบทุกหมวดแล้ว ยกเว้นที่รอตัวติดตาม/ตัวหาใบหน้า และเสียง (ขั้น 10)
- [x] layer styles (9 + Blending Options): Drop Shadow, Inner Shadow, Outer Glow, Inner Glow, Bevel and Emboss (5 แบบ, smooth/chisel),
      Satin, Color Overlay, Gradient Overlay (5 แบบ), Stroke (นอก/ใน/กลาง) — เป็นรายการ layerStyles ต่อจาก transform เรียงตามเมนูเสมอ,
      ทุกค่าเคลื่อนไหว/ใส่ expression ได้ผ่าน prop.*; เงาและแสงนอกวาดใต้ชั้นด้วยโหมดผสมของตัวเอง; ระยะจากขอบแบบตรง
      (Felzenszwalb) สำหรับ spread/choke เส้นขอบ แสง precise และนูน; Global Light หนึ่งเดียวต่อคอมพ์ (แก้ชั้นไหนก็เปลี่ยนทุกชั้น,
      ชั้นที่เพิ่งได้สไตล์รับแสงของคอมพ์); Fill Opacity, ช่องสี R/G/B, knockout; คำสั่ง style.add / style.remove / style.types
- ชื่อที่เป็นแบรนด์ของผู้อื่น (CC…, Lumetri, Keylight, Mocha, Roto Brush) ใช้ชื่อบรรยายของเราเอง

### ขั้น 8: สามมิติ (เสร็จ 2026-10-19 · เทสต์รวม 562 ผ่าน)
- [x] ชั้นกล้องและไฟ: กล้อง (zoom จากเลนส์ mm บนฟิล์ม 36 มม., ระยะชัดลึก, รูรับแสง, iris, highlight) one-node/two-node
      (auto-orient ไปที่ point of interest), ไฟ parallel/spot/point/ambient (กรวย ขอบนุ่ม falloff 3 แบบ เงา ความมืด ความนุ่ม),
      Material Options ทุกชั้นที่มีภาพ (casts/accepts shadows แบบ off/on/only, light transmission, ambient/diffuse/specular,
      shininess, metal); layer.add type camera/light วางให้พร้อมใช้ (กล้องเห็นคอมพ์เท่าตัว, ไฟซ้ายบนหน้าเล็งกลาง)
- [x] ตำแหน่ง 3D: world matrix ทั้งสายแม่ (แม่ 2D นับเป็นแบน), auto-orient along path / towards camera / towards POI,
      กล้องมาตรฐาน 50 มม. เมื่อไม่มีชั้นกล้อง; expression toWorld/fromWorld 3D, toComp/fromComp ผ่านกล้อง, thisComp.activeCamera
- [x] ตัววาด 3D: ชั้น 3D ติดกันเป็นฉากเดียว (2D คั่นเริ่มฉากใหม่); ทุกพิกเซลเก็บชิ้นจากทุกระนาบผ่าน homography ผกผัน (ภาพถูกต้อง
      ทุกมุม) เรียงไกลไปใกล้ตามความลึกจริง ผสมด้วย blend mode และ opacity ของชั้น — ชั้นตัดกันถูกต้อง พิกเซลที่เส้นตัดผ่านดู 4 ครั้ง
      จึงเรียบ; ส่วนที่ไกลจนเล็กกว่าพิกเซลสุ่มหลายจุดไม่กระพริบ; ชั้น 3D หน้าตรงผ่านกล้องมาตรฐานเหมือน 2D ทุกพิกเซล
- [x] แสงและเงา: Blinn-Phong ตามไฟทุกดวง แสงจากหลังผ่านตาม transmission; เงาด้วยการยิงไปหาไฟผ่านชั้นที่ทอดเงา (ชั้นโปร่งให้แสงผ่าน
      มีสีตาม transmission); shadow diffusion = ไฟเป็นแผ่นกลม ยิงจุดกระจายแบบดอกทานตะวัน 4 ลำก่อน ถ้าไม่ตรงกันใช้ 48 (ขอบเงานุ่มเนียน)
- [x] ระยะชัดลึกของกล้อง: เบลอภาพแต่ละชั้นตามความลึกของแต่ละจุด (เลือกระดับเบลอรายพิกเซล), fast rectangle เป็นสี่เหลี่ยม
      นอกนั้นกลม, highlight gain; ตอนเบลอ แต่ละชั้นถูกจัดแสงและเงาในภาพของตัวเองก่อนแล้วจึงเบลอ (เงาเบลอไปด้วย) เฉพาะส่วนที่เห็นได้
- [x] motion blur 3D (ตำแหน่ง กล้อง ไฟ ตลอดชัตเตอร์ วาดภาพชั้นครั้งเดียว), track matte และ preserve transparency ของชั้น 3D,
      comp ที่ collapse และเป็น 3D เข้าร่วมฉากข้างนอก (ทุกชั้นข้างใน ซ้อนกี่ชั้นก็ได้) ถ้าข้างในมีชั้น 2D วาดเป็นแผ่นเดียวพร้อมคำเตือน
- [x] เอฟเฟกต์ที่ใช้ "กล้องของคอมพ์" และ "ไฟดวงแรก" (Card Dance, Card Wipe, Shatter, Caustics) ใช้ได้จริงแล้ว
      — ยังไม่ทำ: iris แบบหลายเหลี่ยมจริง (ตอนนี้กลม), เงาแบบ shadow map สำหรับฉากใหญ่มาก

### ขั้น 9: ส่งออก (เสร็จ 2026-10-19 · เทสต์รวม 590 ผ่าน, เสียงเพิ่มในขั้น 10)
- [x] render queue เก็บในไฟล์โปรเจกต์ (render settings: quality, resolution, span work area/comp/custom, frame rate,
      motion blur / frame blending / effects / solo / guides override, bit depth; output module: format, channels RGB/RGBA/alpha,
      straight/premultiplied, path พร้อม #### ของลำดับภาพ, quality, bitrate, keyframes, ProRes profile, crop, resize, GIF loop)
      คำสั่ง queue.add / set / remove / list / render (บันทึกสถานะ done/failed พร้อมเหตุผล) และ render.export (ทำทันที)
- [x] วิดีโอผ่าน AVFoundation ของระบบ (เรียก Objective-C runtime ตรงจาก Rust): MP4 H.264 High, MP4 HEVC, QuickTime ProRes
      proxy/LT/422/HQ/4444/4444 XQ (4444 เก็บ alpha), แท็กสี BT.709, bitrate/quality/keyframe; ขนาดคี่ตัดให้คู่สำหรับ H.264/HEVC
- [x] ลำดับภาพ PNG (ของเรา), JPEG และ TIFF (ImageIO ของระบบ); GIF ของเราเอง (median cut ต่อเฟรม, error diffusion, โปร่งใส,
      วนซ้ำ, เขียนเฉพาะส่วนที่เปลี่ยน, เวลาแต่ละเฟรมปัดให้ตรงเวลา); ย่อด้วยค่าเฉลี่ยพื้นที่ไม่กระพริบ
- [x] เสียงใน MP4 (AAC) / QuickTime (PCM) ผสมทีละเฟรม และ WAV ของเราเอง (16/24-bit, 32-bit float, mono/stereo, 8–192 kHz)
- [x] Lottie ส่งออกและนำเข้า (crate `lottie`, คำสั่ง lottie.export / lottie.import): คอมพ์ข้างในเป็น precomp asset, ภาพนิ่งฝังในไฟล์
      (data URI) หรือวางข้างไฟล์ใน images/, ชั้น shape / solid / null / comp / image / text / camera, transform (แม่ลูก, แยกมิติ, 3D),
      keyframe (speed/influence ↔ เส้นโค้ง i/o ของ Lottie ทั้งต่อมิติและตามความยาวทางโค้ง, hold, to/ti), ของในชั้น shape ครบ
      (group, rect, ellipse, star, path, fill, stroke+dash, gradient, trim, round, offset, pucker, twist, zigzag, merge, repeater),
      mask, track matte (tt/tp/td), blend mode, time remap, เวลาชั้น, marker, เอฟเฟกต์ที่ player วาดได้ (Gaussian Blur, Tint, Tritone,
      Drop Shadow, Fill, expression controls), ตัวหนังสือเป็นข้อความ (ฟอนต์ + animator + range selector) หรือเป็นโครงตัวอักษร (เส้นโค้งจริง);
      ที่ Lottie เก็บไม่ได้บอกเป็นคำเตือน; `.lottie` อ่านได้ทั้งแบบเก่า (animations/) และใหม่ (a/) ทุกแอนิเมชันในไฟล์ (deflate ของเราเอง)
      ภาพที่หาไม่พบหรืออยู่บนเว็บเก็บเป็นฟุตเทจที่หายไป ไม่ทิ้งชั้น — ตรวจกับ player จริง (lottie-web) แล้วภาพตรงกัน
      ทั้งไฟล์ที่เราส่งออกและไฟล์จริงจากที่อื่นที่เรานำเข้า; ส่งออกแล้วนำเข้ากลับวาดได้พิกเซลเดียวกัน
      — ยังไม่ทำ: layer styles (`sy`) ของ Lottie (player ส่วนใหญ่ไม่วาด), ตัวอักษรแบบฝัง glyph (`chars`), ชั้นเสียงของ Lottie

### ขั้น 10: ฟุตเทจและเสียง (เสร็จ 2026-10-19 · เทสต์รวม 615 ผ่าน)
- [x] crate `os`: ย้ายตัวเขียนวิดีโอ/ภาพจาก export มารวม และเพิ่มตัวอ่าน — ImageIO อ่านภาพทุกแบบที่ระบบอ่านได้ (PNG JPEG HEIC
      TIFF PSD EXR RAW GIF/WebP/APNG เคลื่อนไหวพร้อมเวลาแต่ละเฟรม) เป็น float premultiplied ใน sRGB (เกินขาวได้) หมุนตาม EXIF ครบ 8 แบบ;
      AVFoundation บอกข้อมูลไฟล์ อ่านเฟรมเรียงจากจุดใดก็ได้ (หมุนตามวิดีโอมือถือ, alpha ของ ProRes 4444) และอ่านเสียงเป็น float
      ที่อัตราใดก็ได้; ตัวเขียนวิดีโอรับเสียงด้วย (AAC/PCM) และต่อคิวภาพกับเสียงไม่ให้ค้าง; `say_exceptions` บอกข้อความข้อผิดพลาดของระบบ
- [x] crate `media`: probe (ภาพนิ่ง ภาพเคลื่อนไหว ลำดับภาพที่ตั้งเลข วิดีโอ เสียง), แคชเฟรมตามงบหน่วยความจำ (LRU),
      ตัวอ่านวิดีโอหนึ่งตัวต่อไฟล์ที่ค้างตำแหน่งไว้ (อ่านต่อไม่ต้อง seek), ตีความ alpha straight/premultiplied/ignore, pixel aspect
      เป็นพิกเซลสี่เหลี่ยมจัตุรัส, loops, conform frame rate (เก็บ native rate), เสียงถอดครั้งเดียวต่อไฟล์ต่ออัตรา
- [x] ตัววาดวาดชั้นฟุตเทจ (time remap, stretch, loops) พร้อม frame blending แบบ frame mix และ pixel motion (optical flow ของ Timewarp);
      ไฟล์หายบอกใน notes; คำสั่ง footage.import / set / replace / info และ layer.add type footage
- [x] ตัวผสมเสียง (render `audio`): ทุกชั้นที่มีเสียง (ฟุตเทจ คอมพ์ข้างใน ชั้นที่มี Tone) ตาม Audio Levels (dB, ไล่ระดับในบล็อกไม่เป็นขั้น),
      in/out, solo, audio switch, stretch / time remap / ย้อนกลับ อ่านทีละ sample แบบผสมระหว่าง sample, เอฟเฟกต์เสียงพร้อม pre-roll
      ผสมเป็นบล็อกได้เสียงเดียวกับผสมทีเดียว (พรีวิวกับส่งออกได้เสียงเดียวกัน)
- [x] เอฟเฟกต์เสียง 13 ตัว: Backwards, Bass & Treble, Compressor, Delay, Distortion, Flange & Chorus, Gate, High-Low Pass, Modulator,
      Parametric EQ, Reverb, Stereo Mixer, Tone
- [x] วัดความดังแบบ ITU-R BS.1770 (LUFS integrated + short-term สูงสุด, ผ่านเทสต์ EBU −23 LUFS) peak / RMS dBFS, รูปคลื่น (PNG),
      แปลงเสียงเป็น keyframe (null "Audio Amplitude" มี slider ซ้าย ขวา รวม); คำสั่ง audio.levels / audio.waveform / audio.toKeys
      และเครื่องมือ MCP audio_preview (AI ฟังไม่ได้ จึงดูรูปคลื่นกับตัวเลขแทน) — ตรวจกับคลิปจริงในเครื่องแล้ว
- [x] SVG แบบเวกเตอร์ [เสร็จ: crate svg ของเราเอง (ตัวอ่าน XML เอง, path data ทุกคำสั่งรวม arc, สี CSS 148 ชื่อ/rgb/hsl,
      style sheet แบบ selector ง่าย, linear/radial gradient พร้อม focal/units/transform/spread/href, use/symbol, svg ซ้อน,
      viewBox/preserveAspectRatio, stroke cap/join/miter/dash); นำเข้าเป็น footage วาดคมทุกขนาดตามรายละเอียดที่เห็น;
      layer.shapesFromVector แปลงเป็นชั้นรูปทรง (กลุ่ม path fill stroke gradient ครบ)]
- [x] ชั้นของไฟล์ PSD แยกกัน [เสร็จ: crate psd ของเราเอง (PSD/PSB, ชั้น กลุ่ม หน้ากากชั้น ชื่อ Unicode fill opacity,
      raw/RLE/ZIP/ZIP+prediction, 8/16/32 บิต, RGB/gray/CMYK/Lab, ตัวเขียนส่วนเล็กสำหรับทดสอบ); footage.import as
      composition / compositionLayerSizes: ชั้นละ footage (Footage.layer), กลุ่มเป็นคอมพ์ย่อย pass through ยุบ,
      blend/opacity×fill/ซ่อน/clipping; ตรวจกับ PSD จริงของ macOS ตรงกับภาพรวมของระบบ;
      ภายหลัง: ชั้นตัวหนังสือแก้ได้, layer styles, ชั้นปรับ/ชั้นเติมสี (วาดจากค่าตั้ง), smart object วางตามมุม, vector mask]
- [x] ชั้นเสียงของ Lottie [เสร็จ: ส่งออกเป็นชั้น ty 6 + asset เสียงฝังในไฟล์ (data URI), ระดับเสียง dB เป็น % ของ Lottie;
      นำเข้ากลับเป็น footage เสียง (เก็บไฟล์) และ audio/levels; ต้นแบบไม่มีส่วนนี้ (ของเราเพิ่มเอง)]
- [x] EXR หลายช่อง (ส่งให้เอฟเฟกต์ 3D Channel) [เสร็จ: crate exr ของเราเอง (ย้ายตัวเขียน ZIP มาไว้ด้วย):
      ส่วนหัว/attribute ทุกแบบ, หลาย part, scanline/tile (one level/mipmap/ripmap อ่านชั้นใหญ่สุด), UINT/HALF/FLOAT,
      ช่องที่สุ่มห่าง (ขยายเต็ม), ตารางชังก์เสียก็ไล่หาทีละชังก์, ถอดหลายเธรด; บีบอัด none/RLE/ZIPS/ZIP/PIZ (Huffman +
      wavelet + LUT)/PXR24/B44/B44A/DWAA/DWAB (กฎในไฟล์, DCT + YCbCr, RLE ตามไบต์, ตารางกลับ linear) — ตรวจกับไฟล์ที่
      OpenEXR 3.4 เขียน (ผ่าน OpenImageIO ของ Blender + ภาพเรนเดอร์ของ Blender 5 แบบ part ละ pass) ตรงทุกบิตทุกช่อง
      (DCT ต้องคูณบวกแบบ fused จึงจะได้ค่าเท่าไฟล์จริง), อ่านไฟล์ EXR ทั้ง 86 ไฟล์ในเครื่องได้หมด (ระบบเองอ่าน DWA ไม่ได้);
      ชื่อชั้น part.channel ไม่ซ้ำชื่อ, ภาพหลักของไฟล์หลายชั้น (ชั้นไม่มีชื่อ → beauty ตามชื่อ → ชั้นสีที่ไม่ใช่ data pass),
      Y/RY/BY เป็น RGB ตาม chromaticities; media: ภาพ EXR ด้วยตัวอ่านของเรา (ตรงกับ ImageIO เมื่อถือ alpha เหมือนกัน;
      ค่าเริ่มต้น premultiplied ตามรูปแบบไฟล์, primaries แปลงเป็น 709), passes(path) พร้อม manifest Cryptomatte;
      host.passes() ของชั้นฟุตเทจ EXR (ตามเฟรมของ sequence); ชื่อช่องของ Blender (Depth.Z ก่อน Z อื่น, Object/Material
      Index); passes จากคอมพ์ (ชั้นคอมพ์ที่มีเอฟเฟกต์ 3D Channel): ฉาก 3D ของเราบันทึกชิ้นต่อพิกเซล (run_3d record:
      ระนาบ ตัวอักษร/รูปทรงนูน primitive โมเดลพร้อม normal/uv/วัสดุของโมเดลจาก Shot.facts) ชั้น 2D นอนบนระนาบคอมพ์
      ชั้นบนอยู่หน้า แล้วรวมจากหน้า → Z, ObjectID (ลำดับชั้น), MaterialID (id ของ source), Coverage, N.X/Y/Z (หันหากล้อง),
      UV, CryptoObject (ชื่อชั้น) / CryptoMaterial (ชื่อ source หรือชื่อวัสดุของโมเดล) 4 อันดับ + manifest;
      ภายหลัง: ไฟล์ deep, HTJ2K]
- [ ] ภายหลัง: PDF/AI แบบเวกเตอร์ (ตัวติดตาม: Warp Stabilizer และ Rolling Shutter Repair ทำแล้วในขั้น 11;
      3D Camera Tracker, Object Matte, เอฟเฟกต์ใบหน้า ดู "งานที่เหลือ" ท้ายไฟล์)

### ขั้น 11: เติมคำสั่งให้ครบ (373 คำสั่ง; ส่วนที่เป็นหน้าต่าง/การเลือก/การเลื่อนดู ไม่ทำ)
- [x] รายการในโปรเจกต์: โฟลเดอร์ใหม่ เปลี่ยนชื่อ/ป้ายสี/หมายเหตุ/ย้ายโฟลเดอร์ ทำสำเนาคอมพ์ ลบฟุตเทจที่ไม่ได้ใช้
      นำเข้าโปรเจกต์อื่นมารวม รวบรวมไฟล์ (collect files) โหลดฟุตเทจใหม่
      (project.folder.new, project.item.set/duplicate/remove, project.removeUnused, project.import, project.collect, footage.reload)
- [x] ผู้ช่วย keyframe: Wiggler, Smoother, Motion Sketch (จากเส้นทางที่ให้มา), ย่อ/ขยายเวลาของ keyframe ชุดหนึ่ง,
      เรียงชั้นต่อกัน/เหลื่อมเวลา (sequence layers, stagger), easy ease เข้า/ออก, toggle hold
      (key.wiggle, key.smooth, motion.sketch, key.transform, key.exponentialScale, layer.sequence, layer.stagger, key.ease)
- [x] เวลาและชั้น: slip, ยืดให้ in/out ตรงเวลา, ย้าย in มาที่จุดเริ่ม/out ไปที่ท้าย, freeze ที่เฟรมสุดท้าย,
      ตัดคอมพ์ตาม work area, lift/extract work area, ครอปคอมพ์ตามพื้นที่/ขอบชั้น, poster time
      (layer.slip, layer.stretchTo, layer.freezeLast, comp.trimToWorkArea/liftWorkArea/extractWorkArea, comp.crop, comp.set posterTime)
- [x] หน้ากากและเส้นทาง: แก้จุด (เพิ่ม ลบ แทรก ย้าย เปลี่ยนชนิดจุด เปิด/ปิด), smart mask interpolation, แปลงรูปทรงเป็น bezier,
      รวม/แยกกลุ่ม shape, สร้าง null จากเส้นทาง, auto-trace (หน้ากากจาก alpha/ความสว่าง)
      (path.vertex, mask.interpolate, shape.toPath, shape.group/ungroup, path.nulls, layer.autoTrace;
      crate `path`: แบ่งโค้ง วัดความยาว morph แบบ Procrustes, fit โค้งกำลังสาม, marching squares)
- [x] จัดวาง: align / distribute ชั้น (ตามสิ่งที่ชั้นวาดจริง เทียบคอมพ์หรือกันเอง), nudge (layer.align/distribute/nudge)
- [x] เอฟเฟกต์: ทำสำเนา, รีเซ็ต, เปิด/ปิด, คัดลอกไปชั้นอื่น; animation preset (บันทึก/ใช้), preset ที่มากับโปรแกรม
      (effect.duplicate/reset/set/removeAll; anim.presets/applyPreset/savePreset: preset เป็น JSON ของเรา
      ค่าคิดจากชั้นที่ใส่ เช่น ค่าเดิม คูณ บวก "พ้นขอบซ้าย" หรือสัดส่วนของขนาดชั้น ; ส่ง preset ทั้งก้อนมาเลยก็ได้;
      มากับโปรแกรม 70 แบบ: เข้า/ออก (fade, slide 8 ทิศ, pop, spin, zoom, blur, drop), ทรานซิชัน 22 แบบจากเอฟเฟกต์ของเรา
      (wipe 4 ทิศ, กวาดเฉียง, นาฬิกา, พัด, iris, มู่ลี่, บล็อก, ประตูตาราง, เลนส์, เส้นกวาด, ฟัน), วนซ้ำ 5 แบบ,
      ตัวหนังสือ 8 แบบ, พื้นหลังเคลื่อนไหว 18 แบบ (ไล่สี, ออโรรา, หมอก, เมฆ, ควัน, ถ่านไฟ, พลาสมา, เซลล์, คริสตัล,
      โมเสก, ผ้าไหม, พิมพ์เขียว, หมากรุก, วงคลื่น, สปอตไลต์, ฟ้ายามเช้า, แสงกวาด, เกรน) — ตรวจด้วยภาพทุกแบบแล้ว)
- [x] pre-render (ทำคอมพ์เป็นไฟล์แล้วใช้แทน), scene edit detection (แบ่งวิดีโอเป็นชั้นตามจุดตัด)
      (comp.preRender: import / import & replace usage (ตรงเวลาแม้เรนเดอร์แค่ work area) / set proxy;
      queue item มี after; proxy ของคอมพ์และฟุตเทจ: project.proxy.set/create, render settings proxies;
      layer.sceneEdits: markers / split / split & precompose — ตรวจกับวิดีโอจริงของเราแล้ว)
- [x] essential properties (คอมพ์เปิดค่าบางค่าให้ชั้นคอมพ์ข้างนอกตั้งเองได้): control, กลุ่ม, หมายเหตุ, mirror,
      แทนสื่อ (media replacement), ส่งค่ากลับต้นทาง/คืนค่า, template (ส่งออก/นำเข้า)
      (essential.* 19 คำสั่ง; ชั้นที่แสดงคอมพ์มีกลุ่ม essentials/<ชื่อ> ใช้ prop.set/prop.keys/expression ได้เหมือนค่าอื่น;
      ค่าที่ตั้งตามไปถึงภาพและเสียง, เก็บในไฟล์, ติดไปกับการทำสำเนาชั้น/คอมพ์และการรวมโปรเจกต์; font อย่างเดียว,
      scale เป็นค่าเดียว, link ค่าอื่นให้ตาม)
- [x] ตัวหนังสือให้ครบ (เทียบแล้วยังขาดมาก; ที่เหลือคือ "ภายหลัง" ด้านล่าง): style ทีละช่วงตัวอักษร (ฟอนต์ น้ำหนัก ขนาด สี เส้น tracking leading
      baseline shift, ย่อ/ขยายแนวนอน-ตั้ง, tsume, faux bold/italic, all caps/small caps, ตัวยก/ตัวห้อย, kerning
      metrics/optical/กำหนดเอง, ligature, OpenType features, แกนของ variable font); ย่อหน้า (จัดชิดแบบ justify ทุกแบบ,
      เยื้อง, ระยะก่อน/หลัง, ขวาไปซ้าย, ตัดบรรทัดแบบ every-line, hanging punctuation); ตัวหนังสือแนวตั้ง;
      animator ให้ครบ (3D, line anchor, line spacing, character offset/value/range/alignment, blur, hue/sat/bright/opacity
      ของ fill และ stroke, tracking type) + expression selector + wiggly แยกมิติ; text on path; ลำดับ fill/stroke
      และ inter-character blending; คำสั่งแก้ข้อความเป็นช่วง (แทรก/ลบพร้อม style), รายชื่อฟอนต์/OpenType/แกน
      ลำดับงาน: (1) shaper เลือก feature ได้ + อาหรับ (รูปต่อ isol/init/medi/fina, rlig, curs) + ฮีบรู, ขวาไปซ้าย
      [เสร็จ: ตรงกับ CoreText ทั้ง Arial, SF Arabic (variable + delta ของ GPOS) และฮีบรูที่มีจุด];
      (2) bidi (UAX #9) ของเราเอง [เสร็จ: ตรงกับ ICU ของระบบ 9,000 กรณี]; (3) โมเดล TextDoc: runs, ย่อหน้า,
      ค่าใหม่, แก้ข้อความเป็นช่วง [เสร็จ]; (3b) ฟอนต์: เส้น CFF/CFF2 (ไฟล์ .otf ยังวาดไม่ได้เลย), ขอบเขต glyph,
      ตำแหน่งขีดเส้นใต้ (post), ระยะแนวตั้ง (vhea/vmtx) [เสร็จ: ตรงกับ CoreText ทั้ง Hiragino, Apple SD Gothic Neo,
      LastResort]; ฟอนต์สี [เสร็จ: ภาพ sbix ของ Apple Color Emoji (เลือกขนาดใกล้สุด, dupe) วาดผ่านการเปลี่ยนรูปของตัวอักษร
      และ animator, ชั้นสี COLR v0 + จานสี CPAL (ชั้นไม่มีสีใช้สีของข้อความ), emoji หาฟอนต์สีเอง, ตัวต่อ ZWJ/FE0F/สีผิว/ธง/keycap
      อยู่ในฟอนต์เดียวกันเพื่อให้ morx รวมเป็นภาพเดียว]; ภายหลัง: COLR v1 (ไล่สี) และ CBDT;
      (4) จัดวางใหม่ทั้งหมด [เสร็จ]; (5) เรนเดอร์ + animator ครบ + text on path + fill/stroke order [เสร็จ];
      (6) คำสั่ง [เสร็จ: text.set ทุกค่าทีละช่วง, text.insert/delete, text.fonts, text.fontFeatures,
      text.removeAllAnimators]; (7) ตัวหนังสือ 3D รายตัว [เสร็จ: ผ่านกล้องและแสงเหมือนชั้น 3D]; (8) ตาราง AAT (morx/kerx) สำหรับฟอนต์ของ Apple ที่ไม่มี OpenType (Geeza Pro ฯลฯ) [เสร็จ: morx ครบ 5 แบบ,
      kerx/kern แบบ Apple (คู่, เครื่องสถานะ, ยกข้ามบรรทัด, ankr), trak ตามขนาด + optical size อัตโนมัติ,
      feature ผ่าน setting ของ Apple; ตรงกับ CoreText 203/203 กรณีใน 55 ฟอนต์ และ feature 28 กรณี];
      อักษรอินเดีย [เสร็จ: Indic shaping ของเราเอง 9 อักษร (Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu,
      Kannada, Malayalam) ทั้งกฎใหม่ (dev2…) และเก่า (deva…): แยกสระสองส่วน, ตัดพยางค์, หา base จากที่ฟอนต์จะประกอบ,
      reph, สระหน้า, pre-base form, half form, ZWJ/ZWNJ, วงกลมจุดให้เครื่องหมายลอย, feature ทีละตัวตาม mask ของแต่ละ glyph;
      Khmer และ Myanmar ก็เช่นกัน; ฟอนต์ที่มีทั้ง morx และ OpenType ใช้ morx (และ kerx) แบบ CoreText;
      ตรงกับ CoreText ทุกคำที่ทดสอบ 500 คำใน 27 ฟอนต์ (เว้นเครื่องหมายลอยตัวเดียวต้นข้อความในฟอนต์ AAT)]
- [x] คีย์และค่า: คัดลอกคีย์ไปค่าอื่น/ชั้นอื่น (และกลับด้าน), property link (pick whip), แปลง expression เป็นคีย์,
      reset ค่า, แก้กลุ่มใดก็ได้ (ทำสำเนา ย้าย เปลี่ยนชื่อ ลบ เปิด/ปิด), แก้/แปลง marker, marker ของคอมพ์ซ้อน,
      รายการ expression ที่ผิด, ดูดสีจากชั้น (pick color ให้เอฟเฟกต์)
      [เสร็จ: key.copy, prop.link (เขียนแบบ pick whip แล้วลองคำนวณจริง ถ้าไม่ได้ใช้แบบเรียกทีละชั้น), prop.bake,
      prop.reset (ค่าเริ่มของชั้นนั้นจริง, ทั้งกลุ่มได้), prop.animate (นาฬิกาจับเวลา), group.duplicate/remove/rename/
      enable/move (ย้ายเข้ากลุ่มอื่นได้), marker.set/convert/nested/fromSource (+ cue point, frame target),
      expr.errors, expr.reference (ตัวอย่างทุกอันรันในเทสต์), effect.pickColor; expression รู้จัก content() ของกลุ่ม;
      quick offset มีแล้วใน layer.stagger]
- [x] ชั้นและหน้ากาก: anchor กลางเนื้อชั้น, transform (reset/กลางคอมพ์/fit/fit กว้าง/fit สูง/กลับด้าน), marker จากต้นทาง,
      sequence layers แบบมี dissolve, feather falloff และ motion blur ของแต่ละหน้ากาก, free transform จุด,
      จุดขนนกแปรผัน (variable feather), RotoBezier, taper/wave ของเส้น
      [เสร็จ: layer.transform, layer.centerAnchor (เนื้อชั้นจริงรวมตัวหนังสือ/shape, ชั้นไม่ขยับ), marker.fromSource,
      layer.sequence มี dissolve อยู่แล้ว, หน้ากากมี motionBlur/featherFalloff/rotoBezier, mask.feather (จุดขนนกอยู่บน
      path ไปกับคีย์, ทางลาดเดียวต่อเนื่องข้ามขอบ), mask.shape, mask.reset, path.transform, taper/wave ของ stroke และ
      gradient stroke (+ ใช้กับเส้นประ), marker.remove all; ShapePath เก็บจุดขนนกแบบไม่กินที่เมื่อไม่มี]
- [x] ฟุตเทจและโปรเจกต์: รายการไฟล์หาย + relink (หาไฟล์อื่นในโฟลเดอร์เดียวกันให้ด้วย), reload ที่เปลี่ยน, placeholder และ
      แทนด้วย placeholder/สีพื้น/คอมพ์, รวมฟุตเทจซ้ำ, reduce project, นำเข้าหลายไฟล์, increment and save,
      แทรกช่วงฟุตเทจลงคอมพ์ (overlay / ripple insert)
      [เสร็จ: footage.missing, footage.relink (ไฟล์ที่อยู่ข้างกันตามไปด้วย), footage.find (ค้นโฟลเดอร์ ตรวจขนาด/ชนิด/
      ความยาว), footage.reload changed (pool จำเวลาแก้ไฟล์ตอนอ่าน), footage.placeholder + แถบสีสำหรับ placeholder และ
      ไฟล์ที่หาย, footage.replace placeholder/solid, footage.interpretLike, project.consolidate, project.reduce,
      project.item.usage, footage.import paths, project.save increment/copy, layer.insertFootage (overlay/ripple
      ตัดชั้นที่คร่อมจุดแทรก); ไม่ทำ: watch folder (ฟาร์มเรนเดอร์)]
- [x] เรนเดอร์: หลาย output module ต่อรายการ, template ของ render settings/output module, ทำสำเนา/ย้ายรายการ, EXR
      (และเฟรม EXR หลายชั้น), เฟรมเป็น PSD แยกชั้น, ส่งออก/นำเข้าไทม์ไลน์ (FCPXML, FCP7 XML, OTIO, EDL),
      composition preset (บันทึก/ลบ/รายการ), responsive design (ช่วง intro/outro ที่ไม่ยืด)
      [เสร็จ: RenderItem.extra + queue.output (วาดเฟรมครั้งเดียวส่งทุก module), queue.templates/saveTemplate/
      deleteTemplate (+ settingsTemplate/outputTemplate, เก็บใน Templates.json), queue.duplicate/move, exrSequence และ
      psdSequence, render.frame ตามนามสกุล (.png .jpg .tif .exr .psd; EXR หลายชั้นด้วย layers) — ตัวเขียน EXR (ZIP) และ
      PSD เป็นของเราเอง ระบบอ่านกลับได้, crate interchange (EDL/OTIO/FCP7 XML/FCPXML อ่าน-เขียน, XML ของเราเอง) +
      timeline.export/import (placeholder สำหรับไฟล์ที่หาย), comp.new preset + comp preset ที่บันทึก,
      comp.responsiveTime + เวลาของคอมพ์ซ้อนที่ยืดโดยช่วงที่ป้องกันเล่นความเร็วเดิม]
- [x] กล้องและไฟ: dolly/pan/orbit, โฟกัสไปที่ชั้นหรือจุดเล็ง (ตั้ง/ผูก), orbit null, stereo rig, ไฟตามกล้อง
      [เสร็จ: camera.orbit (yaw รอบแกนตั้งของโลก, pitch ไม่เกิน 89°), camera.pan, camera.dolly (still: จุดเล็งอยู่กับที่),
      camera.lookAt (หันไปกลางกล่องของชั้น 3D แล้วถอยให้พอดีภาพตามขอบที่เว้น หรือ keepDirection เลื่อนข้าง), camera.focus
      (ที่ชั้นตามแนวมองหรือที่จุดเล็ง, link ด้วย expression), camera.orbitNull, camera.stereoRig (เอฟเฟกต์ Stereo 3D Controls
      ใหม่, คอมพ์ตาซ้าย/ขวาที่กล้องตามกล้องหลักด้วย expression รวมการเอียง roll, คอมพ์ผลด้วย 3D Glasses), light.followCamera;
      กล้อง one-node หมุน orientation ตาม, กล้องที่มีแม่เขียนกลับในพื้นที่ของแม่, มีคีย์แล้วใส่คีย์ที่เวลานั้น;
      expression เพิ่ม toWorldVec/fromWorldVec/toCompVec/fromCompVec และ cameraOption/lightOption;
      กล้องและไฟจากโมเดลทำแล้วพร้อมโมเดล 3 มิติ; environment light ทำแล้ว (ดูหัวข้อโมเดล); ยังไม่ทำ: camera.fromView (เป็นหน้าต่าง)]
- [x] พู่กันและหุ่น: เส้นพู่กัน (brush/clone/eraser บน Paint), บันทึกการลากหมุดหุ่น, follow-through
      [เสร็จ: เอฟเฟกต์ Paint ใหม่ (effects/strokes.rs: เส้น brush/clone/eraser เป็นรายการในเอฟเฟกต์ แต่ละเส้นมี path,
      start/end, สี, ปลายพู่กัน diameter/angle/hardness/roundness/spacing, opacity/flow, channels rgba/rgb/alpha, โหมดผสม 22
      แบบ, แรงกดต่อจุด (ขนาด/flow/opacity), transform ของตัวเอง, from/length ของเวลา; clone จากชั้นนี้ที่วาดถึงตอนนั้นหรือ
      ชั้นอื่น เวลาเลื่อนหรือล็อกเวลา; eraser ลบชั้นและสี/เฉพาะสี/เฉพาะเส้นก่อนหน้า; paint on transparent), paint.stroke
      (constant/writeOn/singleFrame/custom, smooth, tip), paint.tips, ลบเส้นด้วย group.remove; puppet.record (ลากหมุดตามเวลา
      เป็นคีย์ทุกเฟรมแล้ว smooth, ลากหมุดอื่นตามไปด้วย, speed) ใช้แกนเดียวกับ motion.sketch, puppet.follow (expression
      valueAtTime ไล่ตามผู้นำ, cascade ตามระยะ); ไม่ทำ: ค่าในแผง Paint/Brushes และ puppet record options (เป็นหน้าต่าง)]
- [x] ตัวติดตามการเคลื่อนไหว: point tracker (ตำแหน่ง/หมุน/ย่อขยาย) ใช้กับชั้นหรือจุดของเอฟเฟกต์, stabilize, Warp Stabilizer,
      Rolling Shutter Repair; ภายหลัง: 3D Camera Tracker, planar
      [เสร็จ: crate track ของเราเอง (NCC บนพีระมิด + Lucas–Kanade ถึงเศษพิกเซลพร้อม gain/bias และหมุน/ย่อขยายของแพตช์,
      4 แบบเมื่อไม่แน่ใจ, KLT หลายจุด + corner Shi–Tomasi + ตรวจตามกลับ, RANSAC เลื่อน/คล้าย/เอียง/perspective, region tracker),
      motionTrackers บนชั้น footage/comp (project/tracking.rs), track.new/analyze/apply (transform ที่ตำแหน่งหรือจุดของเอฟเฟกต์,
      หมุนและย่อขยายด้วยสองจุด, stabilize, corner pin แบบ parallel/perspective), mask.track (4 แบบ), เอฟเฟกต์ Warp Stabilizer
      (smooth/no motion, position/คล้าย/perspective, crop/crop+auto scale สูงสุดตามที่ตั้ง, เส้นทางกล้องสะท้อนที่ปลาย) และ
      Rolling Shutter Repair (อัตราและทิศการสแกน) ที่ effect.analyze เก็บการเคลื่อนของกล้องไว้; ภายหลัง: Subspace Warp,
      synthesize edges, pixel motion ของ rolling shutter, 3D Camera Tracker, planar]
- [x] ตัวหนังสือ 3D รายตัวอักษร (per-character 3D) [เสร็จ]
- [x] โมเดล 3 มิติ (glTF/OBJ) — งานใหญ่ ทำหลังสุด
      [เสร็จ: crate model ของเราเอง (glTF 2.0 .gltf/.glb: buffer ใน data URI/ไฟล์ข้างๆ/chunk, accessor ทุกชนิดรวม sparse,
      primitive แบบ triangles/strip/fan, วัสดุ metallic-roughness พร้อมรูป base/MR/normal/occlusion/emissive, sampler,
      node tree, กล้อง, ไฟ KHR_lights_punctual, unlit, emissive strength, skin, morph target, animation step/linear/cubic;
      OBJ/MTL: วัตถุ/กลุ่ม/วัสดุ, ดัชนีลบ, สีจุด, Kd/d/Tr/Ke/Ns/Pr/Pm/map_Kd/map_Ke), os อ่านรูปจากไบต์ในหน่วยความจำ,
      footage ชนิด model + ชั้น 3D model (Source::Model, model/unitScale, animation, speed, animationOffset, loop),
      render/meshes.rs วาดสามเหลี่ยมเอง 4 จุดต่อพิกเซล ตัดที่ระยะใกล้ จัดลำดับความลึกกับชั้นแบนรายพิกเซล
      แสงของฉาก+เงาจากชั้นแบน ไฟมาตรฐานเมื่อไม่มีไฟ, model.info, camera.fromModel, light.fromModel;
      เพิ่มแล้ว: โมเดลทอดเงา (ต้นไม้กล่องของสามเหลี่ยม ให้ทั้งชั้นแบนและตัวเอง; castsShadows only ทอดเงาอย่างเดียว),
      depth of field บนโมเดล (เบลอตามความลึกรายพิกเซล ลามออกนอกขอบ), ชั้นตัวหนังสือ/รูปทรงแบบนูน 3D
      (geometryOptions: bevelStyle none/angular/concave/convex, bevelDepth, holeBevelDepth, extrusionDepth; render/extrude.rs
      ตามรอยขอบภาพของชั้นเอง ตัดหน้าเป็นสามเหลี่ยมพร้อมรู (path/caps.rs ear clipping + สะพานรู), bevel เป็นวงที่หดเข้า
      แก้วงที่ไขว้ตัวเอง, ด้านข้างสีตามขอบ, หน้า/หลังเป็นภาพของชั้น; แสง เงา depth of field แบบโมเดล);
      ชั้นวัตถุ 3D สำเร็จรูป (Source::Primitive: cube/sphere/plane/torus/cone/cylinder, geometryOptions ตามชนิด,
      materialOptions เพิ่ม baseColor/metallic/roughness/emissive, model/shapes.rs สร้าง mesh, layer.add type primitive +
      layer.newPrimitive), material.set/reset/duplicateAssign, อ่านไฟล์ STL (binary/ข้อความ, สีแบบ Magics และ VisCAM);
      ไฟ environment (LightKind::Environment: source เป็นชั้นใดก็ได้หรือชั้น environment, rotation; ภาพพาโนรามาลดขนาดเป็นขั้น
      ตามความหยาบ + แสงที่ตกกระทบตามทิศ 32×16, split-sum ของ Karis; ให้แสงทั้งชั้นแบนและโมเดล; ชั้น environment ไม่ถูกวาด;
      ฉากหลังท้องฟ้า environmentBackground วาดผ่านกล้องเหมือนไกลไม่สิ้นสุด; layer.environment, light.environmentBackground);
      ภายหลัง: ความโปร่งใสเรียงลำดับในโมเดล, FBX, ชั้นที่ปรากฏในเงาสะท้อนของกันและกัน (appearsInReflections)]
- ไม่ทำ (ต้องใช้ ML / เป็นหน้าต่าง): Roto Brush, Object Matte, ติดตามใบหน้า, Content-Aware Fill,
  viewer/panels/shortcuts/help/prefs/media browser; ภายหลังถ้าคุ้ม: ปลั๊กอินแบบ wasm (ต้องเขียนตัวรัน wasm เอง),
  สคริปต์ JavaScript แบบ app.project, AAF/OMF

### ขั้น 12: เป็นของเราทั้งหมด (เริ่ม 2026-10-20)
เป้าหมาย: ผลลัพธ์**เท่าเดิมหรือดีขึ้น** แต่โค้ดและข้อมูลทุกชิ้นเป็นของเรา ไม่เหลือของใคร แล้วลบ NOTICE ทิ้ง ไลเซนส์ของเรา (LICENSE: สงวนลิขสิทธิ์)

**วิธีทำ (ทุกส่วน)**
1. บันทึกผลลัพธ์ของเดิมเป็นเทสต์อ้างอิงก่อนแตะโค้ด (golden)
   - เอฟเฟกต์: `crates/effects/src/goldens.rs` บันทึก 298 ตัว 4,784 กรณี ไว้ที่ `~/AI_working_spaces/veymelo-goldens/effects`
   - ส่วนอื่น: ทำ golden ก่อนแตะส่วนนั้น บันทึกจาก commit 59dc92b ที่ยังไม่ถูกแก้
2. เขียนใหม่ด้วยวิธีของเราเอง: โครงสร้างใหม่ที่อ่านง่ายกว่าและเร็วกว่า ค่าคงที่และตารางคำนวณหรือพิสูจน์เองจากหลักการ หรือจากมาตรฐานสาธารณะ
3. เทียบ golden: ต้องเท่าเดิม ถ้าต่าง ต้องดูภาพแล้วตัดสินว่าดีขึ้นจริงก่อนยอมรับ
4. ลบรายการนั้นออกจาก NOTICE (`~/AI_working_spaces/veymelo-goldens/notice_drop.py NOTICE "<ต้นบรรทัด>"`)

**ผล (เสร็จ 2026-10-20 · เทสต์ 800 ผ่าน · golden เอฟเฟกต์ 4,784 กรณี และ expression 156 ตัวเท่าเดิมหรือดีขึ้น)**
- [x] keyframe: เทสต์อิทธิพลซ้อนเกิน 100 % ใช้ค่าที่เราคำนวณเอง (ไล่ครึ่งช่วงหาเวลาบนเส้นโค้ง)
- [x] zip: inflate เขียนใหม่แบบตารางค้นสองชั้น เร็วขึ้น 188 → 280 MB/s
- [x] blend modes: อธิบายจากสูตรเองว่าทำไมบางโหมดตัดที่ขาว + เทสต์พิสูจน์
- [x] expr: gaussRandom คำนวณความกว้างจากเปอร์เซ็นไทล์ 95 เอง (ต่างเดิม ≤ 0.00064)
- [x] puppet: น้ำหนักหมุด bend ตั้งชื่อและอธิบายด้วยเทสต์ของเรา
- [x] ตัวหนังสือ: อินเดีย เขมร พม่า ให้ CoreText ของระบบจัด (shape/system.rs; ลบโค้ดแบบ HarfBuzz ทั้งหมด รวม mask/syllable
      ในการจับคู่ GSUB); ตารางจับคู่ฟีเจอร์ Apple ↔ OpenType ทำใหม่จากชื่อในทะเบียนของ Apple (SFNTLayoutTypes.h) + เพิ่ม pkna
- [x] Brightness & Contrast: เส้นโค้งของเราเอง (middle grey 0.8/0.2, slope 1.5/0.5, สว่าง-มืดสมมาตร) ต่างเดิม ≤ 2/255
- [x] Color Grade looks, Lens Flare (แบบจำลองเลนส์ของเรา: ghost จากคู่ผิวกระจก, สีจากเคลือบเลนส์), Add Grain presets
      (ขนาดเม็ดตามขนาดเฟรมฟิล์ม) ออกแบบใหม่ ดูภาพเทียบแล้วเท่าเดิมหรือดีกว่า
- [x] K-weighting (BS.1770): หาตัวกรองแอนะล็อกเองจากค่าที่ ITU ให้ที่ 48 kHz (ตรงมาตรฐานทุกหลัก)
- [x] grain: gain ของ soft linear พิสูจน์จากผลรวมกำลังสองของน้ำหนัก
- [x] ที่เหลือใน NOTICE เป็นพฤติกรรมของเอฟเฟกต์/คำสั่ง (ตามโปรแกรมตัดต่อทั่วไป), รูปแบบไฟล์ตามสเปก และวิธีในตำรา
      ซึ่งโค้ดเป็นของเราอยู่แล้ว: เอา NOTICE ออก, แก้หัว PLAN.md และ README
- [x] commit (a364994)

### ขั้น 13: 3D ระดับ 3 (เริ่ม 2026-10-20)
เป้าหมาย: 3D ทั้งหมดวาดบน GPU (Metal) เร็วพอพรีวิวสด แล้วใช้ ray tracing ของชิป (M3 ขึ้นไปมีในฮาร์ดแวร์) ให้เงา เงาสะท้อน
และแสงที่บังกันเป็นของจริง ผลต้องเท่าเดิมหรือดีขึ้นเหมือนขั้น 12 (golden ของฉาก 3D: บันทึกจากตัว CPU ก่อน แล้วเทียบ)

ก่อนเริ่ม: GPU (three.metal) วาดได้แค่ชั้นแบน; ฉากที่มีโมเดล, ไฟ environment, โมเดลทอดเงา หรือชั้นเกิน 64 จะกลับไปวาดด้วย CPU
ทั้งฉาก; โมเดลวาดด้วย CPU (meshes.rs: 4 จุดต่อพิกเซล, ทีละแถบ 32 แถว)

- [x] 13.0 golden ของฉาก 3D: ชั้นแบน + ไฟ + เงา, โมเดล/primitive/ตัวหนังสือนูน (โมเดลทอดเงา), environment, DOF, motion blur
      (engine/goldens3d.rs; บันทึกจาก CPU ทั้งขนาด 480×270 และ ×4 = 1080p: `VEYMELO_GOLDENS_CPU=1`, `VEYMELO_GOLDENS_SCALE=4`);
      วัสดุ PBR + รูปทุกแบบ เทียบ GPU กับ CPU ในเทสต์ meshes::tests (ต่างสุด < 1/255)
- [x] 13.1 โมเดลบน GPU: rasterizer ของเราเองใน compute (meshes.metal) แทน render pipeline (ได้ผลตรงของเดิมกว่า:
      4 จุดต่อพิกเซลตำแหน่งเดียวกัน, ลงสีครั้งเดียวต่อสามเหลี่ยมที่กลางพิกเซล, ข้อมูลผิวของจุดที่ใกล้สุด, แสงเดียวกับ
      scene::shade; สามเหลี่ยมแบ่งลงช่อง 32×8 พิกเซลบน CPU); รูปของโมเดลเก็บบน GPU ครั้งเดียวต่อโมเดล (light.metal ใช้ร่วม);
      kernel ฉากรับภาพโมเดลมาเรียงความลึกรวมกับชั้นแบนบน GPU; ชั้น 3D แบบ solid/precomp วาดแค่ส่วนที่กล้องเห็น (เดิมวาดทั้งแผ่นที่
      ความละเอียดสูงสุด); buffer ใหญ่เก็บไว้ใช้ซ้ำโดยจำกัดรวมทุกเธรด 1.5 GB (ลองให้ GPU อ่านหน่วยความจำเดิมตรงๆ แล้ว ช้ากว่าคัดลอก)
      ผล 1080p: primitives 92 → 50 ms, motion blur 1169 → 418 ms; 480×270: ~35 → ~8 ms; ยังเหลือ CPU: โมเดลที่มีเงาตกใส่,
      ฉากที่มีโมเดลทอดเงา (→ 13.3)
- [x] 13.2 ไฟ environment บน GPU (split-sum เดิม ใน light.metal ใช้ทั้งชั้นแบนและโมเดล) และเลิกจำกัด 64 ชั้น: แต่ละพิกเซล
      เก็บรายการชิ้นที่อยู่ตรงนั้นได้ 16 ชิ้น พิกเซลที่มีมากกว่านั้น GPU ปล่อยให้ CPU วาดเฉพาะพิกเซลนั้น (golden "many": 120 ใบ +
      ซ้อนกัน 24 ใบ); แก้บั๊กโมเดลที่ไม่รับแสงในฉากที่มีแค่ environment กลายเป็นสีดำ; 1080p environment 97 → 53 ms
- [x] 13.3 ray tracing ของ Metal (os::gpu `Shapes` = acceleration structure, `Arg::Shapes`; kernel รุ่น RAYS):
      - เงาของโมเดลบน GPU (เดิม CPU ทั้งฉาก): ยิงแสงผ่านสามเหลี่ยมของโมเดลที่ทอดเงา ชั้นแบนยังคิดตรงๆ เหมือนเดิม
        (shadows.metal ใช้ร่วมทั้งสอง kernel); เงาตกบนโมเดลก็บน GPU แล้ว; 1080p ฉากเงาโมเดล 275 → 64 ms
      - ambient occlusion (ใหม่): ไฟ ambient/environment มี Ambient Occlusion + Occlusion Distance; 32 ทิศจากแต่ละจุด
        ผ่านโมเดลทุกชิ้น (acceleration structure ชุดที่สอง) และชั้นแบน (ตามความโปร่ง); CPU ทำแบบเดียวกัน
      - เงาสะท้อน (ใหม่): Material Options มี Reflection Intensity / Sharpness / Rolloff; สะท้อนโมเดลหรือชั้นแบนที่ใกล้สุด
        (ลงแสงแบบง่าย: ambient + diffuse + แสงที่ environment ตกใส่) หรือ environment; ความคมน้อยกว่า 100% ใช้ 16 ทิศในกรวย;
        สีของโมเดลในเงาสะท้อนอ่านจากรูปที่มุมของสามเหลี่ยม
      - แผ่นรูปของชั้นแบนคัดลอกขึ้น GPU ครั้งเดียวต่อฉาก ใช้ร่วม เงา/โมเดล/ฉาก
      - แก้ตัวหนังสือนูน: bevel ลึกเกินเส้น ทำให้หน้าแตก รูดำ หนามแหลม (ตัว A, e, m, l): จำกัด bevel ให้เท่าที่เส้นที่บางสุดรับได้
        (ทุกจุดยังอยู่ในเนื้อและห่างขอบอื่นพอ, วงไม่ตัดกัน, ไม่บีบขาดเป็นสองวง; เหลือหน้าอย่างน้อยเท่า bevel) ใช้ค่าเดียวทั้งชั้น;
        untangle เลือกห่วงที่จะตัดตามทิศ (ห่วงกลับด้าน) แทนจำนวนมุม
      - golden ใหม่: occlusion, mirror (CPU กับ GPU ต่างแค่ขอบ/เม็ดสุ่ม); showcase_3d (1080p GPU 372 ms, CPU 5.7 s)
      - ยังเหลือ: โมเดลมีรูปในเงาสะท้อนอ่านแค่ที่มุม (เม็ดในเงานุ่ม แก้ใน 14.4; สะท้อนชั้นเดียว → 3 ชั้นใน 14.2)
- [x] 13.4 พรีวิว 3D สดในแอป: ลากบนพรีวิวของคอมโพที่มีชั้น 3D เพื่อหมุนรอบสิ่งที่กล้องมอง (เหมือนหมุนด้วยมือ),
      เลื่อนขึ้นเข้าใกล้; ระหว่างขยับวาดครึ่งความละเอียด ไม่มี motion blur แล้วขยายเต็ม (หยุด 0.3 วิ วาดเต็ม);
      ปุ่ม "Reset view" กลับกล้องของคอมโพ; คำใบ้ "Drag to look around · scroll to move in" เมื่อชี้; โปรเจกต์ไม่เปลี่ยน
      (render::Options::orbit → Eval::orbit ใช้กับคอมโพที่ดูอยู่เท่านั้น); เทสต์ภาพ studio::watching::a_3d_preview_looks_round

### ขั้น 14: 3D ระดับ 4 (เริ่ม 2026-10-21)
เป้าหมาย: แสงแบบ path tracing — แสงสะท้อนต่อทอด, แก้วหักเห, เงาสะท้อนซ้อนกัน, เงานุ่มไม่มีเม็ด; ผล GPU ตรงกับ CPU
(golden CPU ใน cpu/ เทียบด้วย `VEYMELO_GOLDENS_VS_CPU=1`)

- [x] 14.1 แสงสะท้อนต่อทอด (global illumination): คอมโพมี Light Bounce (comp.set `lightBounce` %, `bounces` 1–4; ไฟล์ `lightBounce`)
      เก็บแสงบนผิว (bounce.rs): ตารางจุดบนชั้นแบนแต่ละชั้น (40 ช่องด้านยาว) + ทุกจุดยอดของโมเดล, จุดละ 128 ทิศ
      (ชนผิวที่ใกล้สุด ลงแสงแบบง่ายพร้อมเงา 1 เส้นต่อไฟ + ท้องฟ้าของ environment), ทำซ้ำตามจำนวนทอด, ตารางเกลี่ย 1-2-1;
      kernel `gather` (shadows.metal) และ CPU แบบเดียวกัน; golden "bounce" (กล่อง Cornell): 1080p GPU 167 ms, CPU 908 ms
- [x] 14.2 เงาสะท้อนซ้อนกัน: สิ่งที่เห็นในกระจกที่สะท้อนได้ก็สะท้อนต่อ ลึกสุด 3 ชั้น (`meet` + รวมจากชั้นท้ายกลับมา)
- [x] 14.3 แก้ว: Material Options มี Transparency / Transparency Rolloff / Index of Refraction (โมเดล);
      หักเหเข้า เดินในเนื้อ ออกและหักเหอีกรอบ (สะท้อนกลับหมดภายในได้ 4 ครั้ง) แล้วเห็นฉากด้านหลัง ย้อมด้วยสีแก้ว;
      ไม่เจออะไรด้านหลัง = ให้สิ่งที่อยู่ใต้โมเดลโผล่ผ่าน; golden "glass"
- [x] 14.4 เงานุ่มไม่มีเม็ด: ตรวจกลางแผ่นแสง + 8 จุดรอบขอบก่อน (เดิม 4 จุด พลาดขอบเงาเป็นจุดขาว), ขอบเงาใช้ 128 จุด (เดิม 48)
- [x] 14.5 สิ่งที่เห็นในกระจกและผ่านแก้วได้แสงสะท้อนต่อทอดด้วย; showcase ห้อง (ลูกแก้ว ลูกโครเมียม แสงสะท้อน AO):
      1080p GPU 0.2 วิ, CPU 4.3 วิ
- [ ] ยังเหลือ: caustics (แสงรวมผ่านแก้ว), เงาของแก้วให้แสงผ่าน, รูปบนโมเดลในเงาสะท้อนอ่านแค่ที่มุม
      (Light Bounce ตั้งผ่านคำสั่ง `comp.set` — ไม่ทำปุ่มในแอปจนกว่าผู้ใช้สั่ง)

### ขั้น 15: 2D บน GPU (เริ่ม 2026-10-21)
เป้าหมาย: งาน 2D (ตัวกรอง เอฟเฟกต์ การวางชั้น) ทำบน GPU ได้ภาพเท่ากับ CPU (ต่างไม่เกิน 1e-5 ต่อช่องสี): ภาพที่ AI ขอดู
และการส่งออกเร็วขึ้น
วัดด้วย: `frames_take` (engine: โปรเจกต์จริง เล่นต่อ 2 วิจากแต่ละจุด), `effects_take` (effects: ทุกเอฟเฟกต์บนภาพ 1080×1920),
`blur_takes` (raster)

- [x] 15.0 ตัวหนังสือในแอปทุกภาษา: Label (button.rs) เลือกฟอนต์ของระบบตามอักษร (faces::fallback), bidi แยกทีละส่วนระหว่าง " · ",
      shape ทุกช่วง, อีโมจิสี (sbix/COLR), ข้ามอักขระควบคุมที่มองไม่เห็น, จำผลไว้; system::fallback_for ตรวจว่าฟอนต์มีตัวนั้นจริง
      + รายการอักษรเพิ่ม (Georgian, Armenian, Ethiopic "Kefa III", Tibetan "Kokonor" ตามระบบ ฯลฯ) + ค้นทุกฟอนต์ในเครื่องจาก cmap
      เป็นทางสุดท้าย (ตัวหนังสือในวิดีโอได้ด้วย); เทสต์ภาพ studio::watching::many_scripts_show (25 ภาษา), button labels_in_many_scripts
- [x] 15.1 ทางให้ตัวกรองไป GPU: raster/gpu.rs + filter.metal; `gpu::on_processor`/`with_mode` ต่อเธรด (render ตาม Options.gpu,
      ส่งต่อเข้าเธรดของ par); ภาพเล็กกว่า 256×256 ทำบน CPU (`VEYMELO_GPU_FROM`); GPU ล้มครั้งเดียวก็กลับไป CPU;
      os::gpu::run_steps (หลายขั้นในงานเดียว), Arg::Pass, Step.group
- [x] 15.2 Gaussian/กล่อง: ผ่านทีละแถวผ่าน threadgroup memory, กล่องเลื่อนทีละ 8 px, bell ทีละ 4 px, คอลัมน์ = กลับภาพแล้วผ่านแถว;
      2000×2000: σ11 73→17 ms, σ40 56→22 ms (ทั้งหมดรวมคัดลอกเข้าออก)
- [x] 15.3 Directional/Radial/Zoom (filter::along, round) ตรงกับ CPU ≤6e-7; Radial Blur 1080×1920 339→88 ms
- [x] 15.4 จำผลการเติม path (path/fill.rs): ลายนิ้วมือ 128 บิตของจุด กฎ และขนาด, เก็บไม่เกิน 192 MB สองรุ่น; ภาพเท่าเดิมทุกบิต
- ผล picture-memory (1080×1920 60fps) เฟรมเดิมซ้ำ: 5 วิ 43→23, 20 วิ 30→16, 40 วิ 66→40, 60 วิ 105→49, 80 วิ 67→38 ms;
  เล่นต่อ: 23–57 ms ต่อเฟรม
- goldens เอฟเฟกต์รันบน CPU; `VEYMELO_GOLDENS_GPU=1 VEYMELO_GPU_FROM=0` ผ่าน GPU: 4771/4784 เท่าเดิม ที่เหลือคือเศษ float
  ถูกขยายที่ threshold/รอยต่อ (Environment ตรงรอยต่อของแผนที่สะท้อน, Cartoon ขอบ) ; 3D focus x4 เขียนใหม่ (ต่าง ≤1/255)
- [x] 15.5 noise บน GPU: Fractal/Turbulent Noise (noise 32 บิตเหมือน CPU), noise 64 บิตเก็บพิกัดเป็นเลขสองตัว (df) ตรงกับ CPU
      ≤1e-6: Curl Noise, Turbulent Displace, Roughen Edges, VR Fractal Noise; hash แบบ 32 บิตสองซีก (GPU คูณ 64 บิตช้า);
      1080×1920: Fractal 54→12 ms, Curl 329→10, VR Fractal 381→26, Turbulent Displace 44→7, Roughen 82→10
- [x] 15.6 blend modes ทั้ง 38 บน GPU (common.metal) ตรงกับ veymelo_color::blend; ขั้นตอน GPU ต่อกันเก็บภาพในช่อง (slots) ได้
- [x] 15.7 เอฟเฟกต์บน GPU: Radial Fast Blur 510→44, Zoom 64→12, Bilateral 112→10, Smart 20→8, Light Burst 101→12,
      Light Rays 95→6, Cell Pattern 65→13, Shadow/Highlight 55→18 (นับตัวอย่างจากระยะแบบ df จึงเท่า CPU)
- [x] 15.8 แม่นยำบน CPU แต่ทุกคอร์ (ภาพเท่าเดิมทุกบิต): histogram นับเป็นจำนวนเต็ม, matte ของ key ทั้งหมด, min/max
      (van Herk/Gil-Werman), clamp/quantize/settle, พื้นหลัง, เส้นโค้งจองที่ก่อน, ชั้นรูปทรงใหญ่เติมและผสมบนทุกคอร์;
      Color Range 77→18, Refine Soft 72→42, Refine Hard 52→34
- [x] 15.9 ภาพที่ GPU ทำอยู่ในหน่วยความจำที่ใช้ร่วม (raster::Pixels): ขั้น GPU ถัดไปอ่านที่เดิม ไม่คัดลอกไปกลับ;
      3D คัดลอกภาพของชั้นที่ GPU ทำด้วย GPU (blit); เอฟเฟกต์ GPU เร็วขึ้นอีก 10–35%
- ผล picture-memory (เล่นต่อ, ต่อเฟรม): 5 วิ 24→17 ms, 20 วิ 26→22, 40 วิ 37→33, 60 วิ 59→52, 80 วิ 47→41
  (ก่อนขั้น 15: เฟรมเดิมซ้ำ 43/30/66/105/67 ms)
- ไม่ย้าย (เหตุผล): Hair และ Flow Motion (วาดเส้นหรือจำลองตามลำดับ ผลขึ้นกับลำดับ), Remove Grain และ Cartoon (สายขั้นตอนยาว
  ใช้กับฟุตเทจเป็นหลัก — ทำได้ด้วยช่องของ GPU ถ้าต้องการ), VR Converter/Plane↔Sphere/Blur/Glow/Sharpen/De-Noise/Chromatic
  (จำนวนตัวอย่างต่อพิกเซลขึ้นกับมุมที่ต้องคิดแบบ df trig ให้เท่า CPU), การวางชั้น 2D และเติม path บน GPU (ชั้น 2D ส่วนใหญ่
  เล็ก งานกระจายบน CPU แล้ว; เหลือเป็นงานใหญ่: ภาพทั้งเฟรมอยู่บน GPU)

## งานที่เหลือ (จัดใหม่ 2026-10-21)
Veymelo ทำเพื่อ AI: ทุกข้อด้านล่างคือสิ่งที่ AI ใช้ผ่านคำสั่ง/MCP หรือความเร็วและคุณภาพของภาพ — **ไม่มีงานหน้าตาสำหรับคน**
(ปุ่ม แผงตั้งค่า การลากแก้ ฯลฯ) จนกว่าผู้ใช้จะสั่ง เรียงตามลำดับที่จะทำ

1. ~~**2D บน GPU (ขั้น 15)**~~ — เสร็จ 2026-10-21 (ดูขั้น 15; ที่ไม่ย้ายมีเหตุผลอยู่ที่นั่น)
2. **3D ที่เหลือจากขั้น 14**: caustics, เงาของแก้วให้แสงผ่าน (มีสี), รูปบนโมเดลในเงาสะท้อนอ่านทุกพิกเซล
3. **3D รายการเก่า**: ม่านรูรับแสงหลายเหลี่ยม (bokeh), shadow map สำหรับฉากใหญ่มาก, ความโปร่งใสเรียงลำดับในโมเดล, FBX,
   ชั้นที่เห็นกันในเงาสะท้อน (appearsInReflections)
4. **หุ่นเชิด**: mesh ที่จำรูปร่างตอนสร้าง, เส้นตาข่ายบนภาพที่ AI ขอดู, motion blur ของ mesh, ปักหมุดตามเส้นทาง
5. **ตัวติดตาม**: 3D Camera Tracker, planar, Subspace Warp, synthesize edges, pixel motion ของ Rolling Shutter
6. **สีและตัวหนังสือ**: ACES output transforms (RRT+ODT), GradingPrimary/Tone/Curve; ฟอนต์ COLR v1 (ไล่สี) และ CBDT
7. **ไฟล์**: PDF/AI แบบเวกเตอร์; EXR deep, HTJ2K; Lottie (layer styles, ตัวอักษรฝัง, เสียง);
   PSD (ชั้นตัวหนังสือแก้ได้, layer styles, ชั้นปรับ/เติมสี, smart object วางตามมุม, vector mask)
8. **commit** ขั้น 13–15 (push เมื่อผู้ใช้สั่ง)

ไม่อยู่ในแผน:
- หน้าตาสำหรับคน (เช่น ตั้งค่า Light Bounce ในแอป, แผงของฟังก์ชันต่างๆ) — ทำเมื่อผู้ใช้สั่ง
- ต้องใช้ ML: Roto Brush, Object Matte, ติดตามใบหน้า, Content-Aware Fill — บางอย่างทำได้ด้วย Vision ของ macOS ถ้าผู้ใช้ต้องการ
- หน้าต่าง/แผงแบบโปรแกรมต้นแบบ, watch folder; ภายหลังถ้าคุ้ม: ปลั๊กอิน wasm, สคริปต์ JavaScript, AAF/OMF

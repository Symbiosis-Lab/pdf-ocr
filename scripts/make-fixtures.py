#!/usr/bin/env python3
"""Generate the synthetic fixture PDFs in tests/fixtures (deterministic).

Needs: Pillow (scanned page), pypdf (encryption). The text PDF is written by hand.
"""
import os, sys, io
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "tests", "fixtures")
os.makedirs(OUT, exist_ok=True)

def esc(s):
    return s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")

# (font, size, x, y, text)  -- y from the bottom
def page(n, total):
    ops = []
    def t(font, size, x, y, s):
        ops.append((font, size, x, y, s))
    t("F1", 9, 72, 750, "Example Agency Audit Division")          # running header
    if n == 1:
        t("F2", 22, 72, 700, "Audit of Example Program Controls")
        t("F2", 14, 72, 660, "Summary of Findings")
        body = [
            "The audit examined whether the Agency maintained adequate controls over its program",
            "funds during the period under review. We found that several controls were not applied",
            "consistently, and that the respon-",
            "sibility for approvals was not clearly assigned to any single office in the Agency.",
        ]
        y = 636
        for l in body:
            t("F1", 11, 72, y, l); y -= 14
        y -= 10
        t("F1", 11, 72, y, "Auditors recommend that the Agency take the following steps:"); y -= 20
        for i, l in enumerate(["Assign a single office to approve all program payments.",
                               "Reconcile the payment ledger monthly and keep the records.",
                               "Train staff on the revised approval procedure."], 1):
            t("F1", 11, 90, y, f"{i}. {l}"); y -= 16
        y -= 8
        t("F1", 11, 72, y, "Other observations:"); y -= 18
        for l in ["\x95 Several files lacked a signed approval form.", "\x95 Receipts were stored in more than one location."]:
            t("F1", 11, 90, y, l); y -= 16
    elif n == 2:
        t("F2", 14, 72, 700, "Corrective Action Plan")
        y = 676
        for k, txt in enumerate(["Corrective Action #1: The Agency will designate a payment approval office",
                                 "by the end of the next quarter and publish its name internally.",
                                 "Corrective Action #2: The Agency will reconcile the payment ledger every",
                                 "month and retain the reconciliation for seven years."]):
            t("F1", 11, 72, y, txt); y -= 14
            if k % 2 == 1: y -= 10
        y -= 6
        t("F2", 14, 72, y, "Response Schedule"); y -= 22
        t("F1", 11, 72, y, "Item        Owner        Due"); y -= 14
        t("F1", 11, 72, y, "Approval     Finance      March 1"); y -= 14
        t("F1", 11, 72, y, "Ledger       Accounting   April 1"); y -= 14
    else:
        t("F2", 14, 72, 700, "Conclusion")
        y = 676
        for l in ["The Agency has agreed to implement the actions above. We will follow up",
                  "on their status in twelve months."]:
            t("F1", 11, 72, y, l); y -= 14
    t("F1", 9, 260, 40, f"Page {n} of {total}")                    # running footer
    t("F1", 9, 72, 40, "Confidential Draft")
    stream = "BT\n"
    for font, size, x, y, s in ops:
        stream += f"/{font} {size} Tf 1 0 0 1 {x} {y} Tm ({esc(s)}) Tj\n"
    stream += "ET\n"
    return stream

def build_text_pdf(path, total=3, pagefn=None):
    pagefn = pagefn or page
    objs = []
    objs.append("<< /Type /Catalog /Pages 2 0 R >>")
    kids = " ".join(f"{5+2*i} 0 R" for i in range(total))
    objs.append(f"<< /Type /Pages /Kids [{kids}] /Count {total} >>")
    objs.append("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    objs.append("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>")
    for i in range(total):
        pid = 5 + 2 * i
        objs.append(f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {pid+1} 0 R "
                    f"/Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> >>")
        s = pagefn(i + 1, total)
        objs.append(f"<< /Length {len(s.encode('latin-1'))} >>\nstream\n{s}endstream")
    out = bytearray(b"%PDF-1.4\n")
    offs = []
    for i, o in enumerate(objs, 1):
        offs.append(len(out))
        out += f"{i} 0 obj\n".encode() + o.encode("latin-1") + b"\nendobj\n"
    x = len(out)
    out += f"xref\n0 {len(objs)+1}\n0000000000 65535 f \n".encode()
    for o in offs:
        out += f"{o:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {len(objs)+1} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n".encode()
    open(path, "wb").write(out)

def sidebar_page(n, total):
    ops = []
    side = ["Jane Roe", "Chair", "John Doe", "Mary Major", "Commissioners", "Pat Smith", "Executive Director"]
    y = 700
    for l in side:
        ops.append(("F1", 9, 54, y, l)); y -= 22
    body = [("F1", 11, 200, 700, "March 3, 2021"),
            ("F1", 11, 200, 672, "Dear Chair Example:"),
            ("F1", 11, 200, 644, "We write to confirm that the review of the program is complete and that the"),
            ("F1", 11, 200, 630, "results are attached to this letter for your records and for your board."),
            ("F2", 11, 200, 602, "Purpose"),
            ("F1", 11, 200, 588, "The purpose of this review was to evaluate whether the program followed its own"),
            ("F1", 11, 200, 574, "published procedures during the period under review, and to report what we found.")]
    ops += body
    st = "BT\n"
    for font, size, x, y, t in ops:
        st += f"/{font} {size} Tf 1 0 0 1 {x} {y} Tm ({esc(t)}) Tj\n"
    return st + "ET\n"

def _paras(items, top=700, size=11, lead=14, gap=10):
    """items: ('p', [lines]) paragraphs or ('h', text) headings -> content stream."""
    ops, y = [], top
    for kind, v in items:
        if kind == "h":
            ops.append(("F2", 14, 72, y, v)); y -= 26
        else:
            for l in v:
                ops.append(("F1", size, 72, y, l)); y -= lead
            y -= gap
    st = "BT\n"
    for font, sz, x, yy, t in ops:
        st += f"/{font} {sz} Tf 1 0 0 1 {x} {yy} Tm ({esc(t)}) Tj\n"
    return st + "ET\n"

def para_break_page(n, total):
    # page 1 ends mid-sentence, with a word hyphenated at the break
    if n == 1:
        return _paras([("p", ["The review covered every payment made during the year and found that most were",
                              "supported by a signed form. The remaining payments were approved by an admin-"])])
    return _paras([("p", ["istrative office that had no written authority to approve them, and the review",
                          "recommends that the authority be documented."]),
                   ("p", ["A second paragraph on the same page ends with a full stop."])])

def new_para_page(n, total):
    # page 1 ends without punctuation, but page 2 starts a new (capitalised) paragraph
    if n == 1:
        return _paras([("p", ["The review covered every payment made during the year and found that most were",
                              "supported by a signed form, as set out in the following section"])])
    return _paras([("p", ["The next section describes the payments that lacked a signed form and the",
                          "steps taken to find the missing records."])])

def heading3_page(n, total):
    if n == 1:
        return _paras([("h", "Overview"),
                       ("p", ["This report describes how the program spent its funds in the year under review",
                              "and what the auditors found when they checked the records."])])
    if n == 2:
        return _paras([("h", "Findings"),
                       ("p", ["Most payments were supported by a signed form, but a minority were not, and",
                              "the reasons are set out below."])])
    return _paras([("p", ["Recommendations follow from the findings above and are listed in order of",
                          "priority for the Agency."])])

def blank_mid_page(n, total):
    if n == 2:
        return "BT\nET\n"
    return _paras([("p", [["Alpha", "Gamma"][n // 3] + " page text sits here and is long enough to be read as text",
                          "directly from the page without any recognition step being needed."])])

def build_scan2(path):
    from PIL import Image, ImageDraw, ImageFont
    fnt = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 54, index=0)
    pages = []
    for lines in (["Scanned Report to the Commissioner", "",
                   "We write to confirm receipt of the building permit application",
                   "submitted on the first of March. The application is complete and",
                   "the committee will review the file and will send a letter in the"],
                  ["coming weeks to say what happens next.", "",
                   "Sincerely,", "The Permit Office"]):
        img = Image.new("L", (2550, 3300), 255)
        d = ImageDraw.Draw(img)
        y = 400
        for l in lines:
            d.text((300, y), l, fill=0, font=fnt); y += 90
        pages.append(img)
    pages[0].save(path, "PDF", resolution=300.0, save_all=True, append_images=pages[1:])

def build_scan(path):
    from PIL import Image, ImageDraw, ImageFont
    img = Image.new("L", (2550, 3300), 255)
    d = ImageDraw.Draw(img)
    fnt = ImageFont.truetype("/System/Library/Fonts/Helvetica.ttc", 54, index=0)
    lines = ["Scanned Letter to the Commissioner", "",
             "We write to confirm receipt of the building permit application",
             "submitted on the first of March. The application is complete and",
             "will be reviewed within thirty days.", "",
             "Sincerely,", "The Permit Office"]
    y = 400
    for l in lines:
        d.text((300, y), l, fill=0, font=fnt); y += 90
    img.save(path, "PDF", resolution=300.0)

def build_encrypted(src, path):
    from pypdf import PdfReader, PdfWriter
    w = PdfWriter(clone_from=PdfReader(src))
    w.encrypt("secret", algorithm="RC4-128")
    w.write(path)

if __name__ == "__main__":
    build_text_pdf(os.path.join(OUT, "text3.pdf"))
    build_text_pdf(os.path.join(OUT, "sidebar.pdf"), 1, sidebar_page)
    build_scan(os.path.join(OUT, "scan1.pdf"))
    build_scan2(os.path.join(OUT, "scan2.pdf"))
    build_text_pdf(os.path.join(OUT, "para_break.pdf"), 2, para_break_page)
    build_text_pdf(os.path.join(OUT, "new_para.pdf"), 2, new_para_page)
    build_text_pdf(os.path.join(OUT, "heading3.pdf"), 3, heading3_page)
    build_text_pdf(os.path.join(OUT, "blank_mid.pdf"), 3, blank_mid_page)
    build_encrypted(os.path.join(OUT, "text3.pdf"), os.path.join(OUT, "encrypted.pdf"))
    open(os.path.join(OUT, "notpdf.pdf"), "w").write("This is just a text file, not a PDF.\n")

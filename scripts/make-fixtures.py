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

def build_text_pdf(path, total=3):
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
        s = page(i + 1, total)
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
    build_scan(os.path.join(OUT, "scan1.pdf"))
    build_encrypted(os.path.join(OUT, "text3.pdf"), os.path.join(OUT, "encrypted.pdf"))
    open(os.path.join(OUT, "notpdf.pdf"), "w").write("This is just a text file, not a PDF.\n")

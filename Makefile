# SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The manual (Steelbore Standard section 8): make info, make html, make pdf.
# Outputs go to doc/build/. Cargo builds the program; this file only builds
# the documentation. Needs texinfo; the PDF also needs TeX (texlive).

SRC   = doc/vernier.texi
BUILD = doc/build
CSS   = doc/spacecraft.css

all: info html pdf

info:
	mkdir -p $(BUILD)
	texi2any --info --no-split --error-limit=0 -o $(BUILD)/vernier.info $(SRC)

html:
	mkdir -p $(BUILD)
	texi2any --html --no-split --css-include=$(CSS) -o $(BUILD)/vernier.html $(SRC)

pdf:
	mkdir -p $(BUILD)
	texi2pdf --texinfo=@afourpaper --quiet --clean -o $(BUILD)/vernier.pdf $(SRC)

clean:
	rm -rf $(BUILD)

.PHONY: all info html pdf clean

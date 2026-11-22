import { gridDisk, latLngToCell } from "h3-js";
import { describe, expect, it } from "vitest";
import { cellPolygons, fogShapes } from "./hexes";

const shibuya = latLngToCell(35.6595, 139.7005, 9);

describe("fogShapes", () => {
  it("covers the world when nothing is uncovered", () => {
    const { fog, uncovered } = fogShapes([]);
    expect(fog.geometry.coordinates).toHaveLength(1);
    expect(fog.geometry.coordinates[0]).toHaveLength(1);
    expect(uncovered.geometry.coordinates).toHaveLength(0);
  });

  it("cuts one hole for a patch of neighbouring cells", () => {
    const { fog, uncovered } = fogShapes(gridDisk(shibuya, 2));
    expect(uncovered.geometry.coordinates).toHaveLength(1);
    // The world ring plus one hole.
    expect(fog.geometry.coordinates[0]).toHaveLength(2);
  });

  it("fogs the middle of a ring again and ignores broken cell IDs", () => {
    const ring = gridDisk(shibuya, 1).filter((cell) => cell !== shibuya);
    const { fog } = fogShapes([...ring, "not-a-cell"]);
    expect(fog.geometry.coordinates).toHaveLength(2);
  });
});

describe("cellPolygons", () => {
  it("draws each cell once, lifted to the shown resolution", () => {
    const fine = gridDisk(shibuya, 1);
    expect(cellPolygons(fine).features).toHaveLength(7);
    expect(cellPolygons(fine, 5).features).toHaveLength(1);
    expect(cellPolygons(fine).features[0]!.geometry.coordinates[0]).toHaveLength(7);
  });
});

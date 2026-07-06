import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ContextMenu } from "./ContextMenu";

function setup(onClose = vi.fn(), onSelect = vi.fn()) {
  render(
    <ContextMenu
      position={{ x: 10, y: 10 }}
      onClose={onClose}
      items={[
        { label: "Rename", onSelect: vi.fn() },
        { label: "Delete", onSelect, danger: true },
      ]}
    />,
  );
  return { onClose, onSelect };
}

describe("ContextMenu", () => {
  it("renders all items", () => {
    setup();
    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeTruthy();
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeTruthy();
  });

  it("clicking an item fires onSelect and onClose", () => {
    const { onClose, onSelect } = setup();
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Escape closes the menu", () => {
    const { onClose } = setup();
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("ArrowDown then Enter selects the second item", () => {
    const { onSelect } = setup();
    const menu = screen.getByRole("menu");
    fireEvent.keyDown(menu, { key: "ArrowDown" }); // active: 0 -> 1 (Delete)
    fireEvent.keyDown(menu, { key: "Enter" });
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it("an outside mousedown closes the menu", () => {
    const { onClose } = setup();
    fireEvent.mouseDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("aria-activedescendant tracks the active item", () => {
    setup();
    const menu = screen.getByRole("menu");
    // Initially points at first item (index 0).
    expect(menu.getAttribute("aria-activedescendant")).toBe("ctx-item-0");
    // After ArrowDown, should point at second item (index 1).
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(menu.getAttribute("aria-activedescendant")).toBe("ctx-item-1");
  });

  it("focus returns to the opener element when the menu unmounts", () => {
    // Render an opener button separately so it is attached to the document.
    const { unmount: unmountOpener } = render(
      <button data-testid="opener">Open</button>,
    );
    const opener = screen.getByTestId("opener");

    // Focus the opener — this is the element that "triggered" the menu.
    opener.focus();
    expect(document.activeElement).toBe(opener);

    // Now render the ContextMenu (it uses a portal to document.body).
    // On mount the effect captures document.activeElement (opener) then focuses
    // the menu div.
    const { unmount: unmountMenu } = render(
      <ContextMenu
        position={{ x: 10, y: 10 }}
        onClose={vi.fn()}
        items={[{ label: "Action", onSelect: vi.fn() }]}
      />,
    );

    // After mounting the menu should own focus.
    expect(document.activeElement).toBe(screen.getByRole("menu"));

    // Unmounting simulates the menu closing; the cleanup should restore focus.
    unmountMenu();
    expect(document.activeElement).toBe(opener);

    unmountOpener();
  });
});

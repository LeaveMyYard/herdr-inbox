// The token of what was shown. A row open on screen, or under a confirmation, keeps the token it had
// when it was opened: a newer feed changes the book and never this, so the verb is refused. Only the
// operator opening it again, or the mod's own verb, moves it on. A row in the list has no pin: the
// list shows the newest feed, so its token is that feed's.

export class Pins {
  constructor(book) {
    this.book = book;
    this.open = null; // { entry, token } of the row on screen
    this.confirm = null; // the same, of the row under a confirmation
  }

  pinned(entry) {
    if (entry === null || entry === undefined) return null;
    const row = this.book.row(entry);
    return { entry, token: row?.token ?? null, row: row ? JSON.stringify(row) : null };
  }

  // A key: a confirmation closed earlier lets go now. The one that a `y` is closing stays for its verb.
  keyStarts(popup) {
    if (!popup.confirm) this.confirm = null;
  }

  // After the model moved: what it now has open is pinned, as of now.
  follow(popup) {
    const entry = popup.entry ?? null;
    if ((this.open?.entry ?? null) !== entry) this.open = this.pinned(entry);
    const target = popup.confirm?.target.entry;
    if (target !== undefined && this.confirm?.entry !== target) this.confirm = this.pinned(target);
  }

  // The operator has been made to look again at what is on screen.
  look(popup) {
    this.open = this.pinned(popup?.entry);
  }

  token(entry) {
    if (this.open?.entry === entry) return this.open.token;
    if (this.confirm?.entry === entry) return this.confirm.token;
    return this.book.row(entry)?.token ?? null;
  }

  // The row as it was shown, as JSON, or null.
  shown(entry) {
    return [this.open, this.confirm].find(pin => pin?.entry === entry)?.row ?? null;
  }

  // The mod's own verb moved the row on.
  moved(entry, token) {
    for (const pin of [this.open, this.confirm]) {
      if (pin?.entry === entry) Object.assign(pin, { token, row: JSON.stringify(this.book.row(entry) ?? null) });
    }
  }

  // The row on screen is not what it was when it was opened.
  get changed() {
    const row = this.open && this.book.row(this.open.entry);
    return Boolean(row) && row.token !== this.open.token;
  }
}

/*
 * retifs for Haiku: the status and settings utility (retifs-desktop), with
 * the Haiku API. The same three parts as on Linux, Windows and Android:
 * this device (retifs, its storage, libraries, libraries heard on the
 * network), the network (Reticulum's connections) and settings.
 *
 * Everything it shows comes from the shared Rust core through
 * retifs_desktop.h, as JSON, parsed into BMessages with BJson. The core's
 * calls block, so they run in a looper of their own (Worker), which sends
 * the answers to the window.
 */

#include <Alert.h>
#include <Application.h>
#include <Bitmap.h>
#include <Button.h>
#include <CheckBox.h>
#include <Deskbar.h>
#include <LayoutBuilder.h>
#include <ListItem.h>
#include <ListView.h>
#include <MenuItem.h>
#include <MessageRunner.h>
#include <Path.h>
#include <PopUpMenu.h>
#include <Roster.h>
#include <ScrollView.h>
#include <SeparatorView.h>
#include <StringList.h>
#include <StringView.h>
#include <TabView.h>
#include <TextControl.h>
#include <Window.h>

#include <Json.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "retifs_desktop.h"

static const char* kSignature = "application/x-vnd.retifs-desktop";
static const char* kDaemonJob = "x-vnd.retifs-daemon";
static const char* kReplicantName = "retifs";

enum {
	kRefresh = 'rfsh',		// worker: read everything again
	kState = 'stat',		// window: what the worker read
	kAct = 'act_',			// worker: do something ("op" and its arguments)
	kDone = 'done',			// window: an action's result ("error")
	kStart = 'strt',
	kRestart = 'rstr',
	kStop = 'stop',
	kLeave = 'leav',
	kJoin = 'join',
	kOpenLink = 'olnk',
	kEnable = 'enab',
	kDisable = 'disa',
	kRemove = 'remv',
	kAddTcp = 'addt',
	kSave = 'save',
};


static BString
Human(double n)
{
	const char* units[] = { "B", "KB", "MB", "GB", "TB" };
	int i = 0;
	while (n >= 1024 && i < 4) {
		n /= 1024;
		i++;
	}
	BString s;
	if (i == 0)
		s.SetToFormat("%.0f B", n);
	else
		s.SetToFormat("%.1f %s", n, units[i]);
	return s;
}


/*! Parses a string the core returned (and frees it). */
static BMessage
Take(char* json)
{
	BMessage m;
	if (json != NULL) {
		BJson::Parse(json, m);
		rd_free(json);
	}
	return m;
}


/*! An action's answer: "" when it went well, else what went wrong. */
static BString
Result(char* s)
{
	BString r(s != NULL ? s : "");
	rd_free(s);
	return r;
}


/*! A JSON array as BJson gives it: fields named "0", "1", ... */
static int32
Count(const BMessage& array)
{
	return array.CountNames(B_ANY_TYPE);
}


static BMessage
Item(const BMessage& array, int32 i)
{
	BMessage item;
	BString name;
	name << i;
	array.FindMessage(name.String(), &item);
	return item;
}


static BString
JsonString(const char* s)
{
	BString out("\"");
	for (const char* p = s; *p; p++) {
		switch (*p) {
			case '"': out << "\\\""; break;
			case '\\': out << "\\\\"; break;
			case '\n': out << "\\n"; break;
			default: out << *p;
		}
	}
	out << "\"";
	return out;
}


// #pragma mark - Worker


class Worker : public BLooper {
public:
	Worker(BMessenger target)
		:
		BLooper("retifs worker"),
		fTarget(target)
	{
	}

	void MessageReceived(BMessage* message)
	{
		switch (message->what) {
			case kRefresh:
			{
				BMessage state(kState);
				state.AddMessage("status", new BMessage(Take(rd_status())));
				state.AddMessage("reticulum", new BMessage(Take(rd_reticulum())));
				state.AddMessage("heard", new BMessage(Take(rd_heard())));
				state.AddMessage("services", new BMessage(Take(rd_services())));
				state.AddMessage("settings", new BMessage(Take(rd_settings())));
				fTarget.SendMessage(&state);
				break;
			}
			case kAct:
			{
				BString op = message->GetString("op", "");
				BString a = message->GetString("a", "");
				BString b = message->GetString("b", "");
				BString error;
				if (op == "service")
					error = Result(rd_service(kDaemonJob, a.String()));
				else if (op == "join")
					error = Result(rd_join(a.String()));
				else if (op == "open_link")
					error = Result(rd_open_link(a.String()));
				else if (op == "leave")
					error = Result(rd_leave(a.String()));
				else if (op == "enable" || op == "disable")
					error = Result(rd_set_interface(a.String(), op == "enable"));
				else if (op == "remove")
					error = Result(rd_remove_interface(a.String()));
				else if (op == "add_tcp")
					error = Result(rd_add_tcp(a.String(), b.String(),
						message->GetInt32("port", 0)));
				else if (op == "save") {
					error = Result(rd_save_settings(a.String()));
					if (error.IsEmpty())
						error = Result(rd_service(kDaemonJob, "restart"));
				}
				BMessage done(kDone);
				done.AddString("op", op);
				done.AddString("error", error);
				fTarget.SendMessage(&done);
				PostMessage(kRefresh);
				break;
			}
			default:
				BLooper::MessageReceived(message);
		}
	}

private:
	BMessenger fTarget;
};


// #pragma mark - Window


class RetifsWindow : public BWindow {
public:
	RetifsWindow()
		:
		BWindow(BRect(100, 100, 640, 560), "retifs", B_TITLED_WINDOW,
			B_AUTO_UPDATE_SIZE_LIMITS | B_QUIT_ON_WINDOW_CLOSE),
		fSettingsShown(false)
	{
		fTabs = new BTabView("tabs");
		fTabs->AddTab(_BuildOverview());
		fTabs->AddTab(_BuildNetwork());
		fTabs->AddTab(_BuildSettings());

		BLayoutBuilder::Group<>(this, B_VERTICAL, 0)
			.SetInsets(B_USE_WINDOW_SPACING)
			.Add(fTabs);

		fWorker = new Worker(BMessenger(this));
		fWorker->Run();
		fWorker->PostMessage(kRefresh);
		fRunner = new BMessageRunner(BMessenger(fWorker), BMessage(kRefresh),
			5000000);
	}

	~RetifsWindow()
	{
		delete fRunner;
		fWorker->Lock();
		fWorker->Quit();
	}

	void MessageReceived(BMessage* message)
	{
		switch (message->what) {
			case kState:
				_Show(message);
				break;
			case kDone:
			{
				BString error = message->GetString("error", "");
				if (!error.IsEmpty()) {
					BAlert* alert = new BAlert("retifs", error.String(), "OK",
						NULL, NULL, B_WIDTH_AS_USUAL, B_WARNING_ALERT);
					alert->Go(NULL);
				}
				if (BString(message->GetString("op", "")) == "add_tcp"
					&& error.IsEmpty()) {
					fName->SetText("");
					fHost->SetText("");
					fPort->SetText("4242");
				}
				break;
			}
			case kStart:
				_Act("service", "start");
				break;
			case kRestart:
				_Act("service", "restart");
				break;
			case kStop:
				_Act("service", "stop");
				break;
			case kLeave:
				_Act("leave", _Selected(fLibraries, fLibraryNames));
				break;
			case kJoin:
				_Act("join", _Selected(fHeard, fHeardAddresses));
				break;
			case kOpenLink:
			{
				BString link(fLink->Text());
				link.Trim();
				_Act("open_link", link);
				fLink->SetText("");
				break;
			}
			case kEnable:
				_Act("enable", _Selected(fConnections, fConnectionNames));
				break;
			case kDisable:
				_Act("disable", _Selected(fConnections, fConnectionNames));
				break;
			case kRemove:
				_Act("remove", _Selected(fConnections, fConnectionNames));
				break;
			case kAddTcp:
			{
				BMessage act(kAct);
				act.AddString("op", "add_tcp");
				act.AddString("a", fName->Text());
				act.AddString("b", fHost->Text());
				act.AddInt32("port", atoi(fPort->Text()));
				fWorker->PostMessage(&act);
				break;
			}
			case kSave:
				_Act("save", _SettingsJson());
				break;
			default:
				BWindow::MessageReceived(message);
		}
	}

private:
	BView* _BuildOverview()
	{
		fState = new BStringView("state", "Asking retifs…");
		BFont bold(be_bold_font);
		bold.SetSize(bold.Size() * 1.2);
		fState->SetFont(&bold);
		fNode = new BStringView("node", "");
		fUsage = new BStringView("usage", "");
		fUsage2 = new BStringView("usage2", "");
		fLibraries = new BListView("libraries");
		fHeard = new BListView("heard");
		fLink = new BTextControl("Link", "A library's link (retifs:…)", "", NULL);

		BView* view = new BView("This device", 0);
		BLayoutBuilder::Group<>(view, B_VERTICAL)
			.SetInsets(B_USE_DEFAULT_SPACING)
			.Add(fState)
			.Add(fNode)
			.Add(fUsage)
			.Add(fUsage2)
			.AddGroup(B_HORIZONTAL)
				.Add(new BButton("Start", new BMessage(kStart)))
				.Add(new BButton("Restart", new BMessage(kRestart)))
				.Add(new BButton("Stop", new BMessage(kStop)))
				.AddGlue()
			.End()
			.Add(new BSeparatorView(B_HORIZONTAL))
			.Add(new BStringView("l", "Libraries"))
			.Add(new BScrollView("ls", fLibraries, 0, false, true))
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(new BButton("Leave", new BMessage(kLeave)))
			.End()
			.Add(new BStringView("h", "Heard on the network"))
			.Add(new BScrollView("hs", fHeard, 0, false, true))
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(new BButton("Join", new BMessage(kJoin)))
			.End()
			.Add(new BSeparatorView(B_HORIZONTAL))
			.AddGroup(B_HORIZONTAL)
				.Add(fLink)
				.Add(new BButton("Add library", new BMessage(kOpenLink)))
			.End();
		return view;
	}

	BView* _BuildNetwork()
	{
		fReticulum = new BStringView("reticulum", "");
		fConnections = new BListView("connections");
		fName = new BTextControl("Name", "", NULL);
		fHost = new BTextControl("Host", "", NULL);
		fPort = new BTextControl("Port", "4242", NULL);

		BView* view = new BView("Network", 0);
		BLayoutBuilder::Group<>(view, B_VERTICAL)
			.SetInsets(B_USE_DEFAULT_SPACING)
			.Add(fReticulum)
			.Add(new BScrollView("cs", fConnections, 0, false, true))
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(new BButton("Enable", new BMessage(kEnable)))
				.Add(new BButton("Disable", new BMessage(kDisable)))
				.Add(new BButton("Remove", new BMessage(kRemove)))
			.End()
			.Add(new BSeparatorView(B_HORIZONTAL))
			.Add(new BStringView("a", "Connect to another Reticulum node over TCP"))
			.AddGrid()
				.AddTextControl(fName, 0, 0)
				.AddTextControl(fHost, 0, 1)
				.AddTextControl(fPort, 0, 2)
			.End()
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(new BButton("Add", new BMessage(kAddTcp)))
			.End();
		return view;
	}

	BView* _BuildSettings()
	{
		fSettingsView = new BView("Settings", 0);
		BLayoutBuilder::Group<>(fSettingsView, B_VERTICAL)
			.SetInsets(B_USE_DEFAULT_SPACING);
		return fSettingsView;
	}

	/*! The settings controls, made once the core said which there are. */
	void _FillSettings(const BMessage& list)
	{
		BLayoutBuilder::Group<> layout(
			(BGroupLayout*)fSettingsView->GetLayout());
		BGridLayout* grid = new BGridLayout();
		layout.Add(grid);
		for (int32 i = 0; i < Count(list); i++) {
			BMessage s = Item(list, i);
			BString key;
			key << s.GetString("section", "") << "." << s.GetString("key", "");
			const char* value = s.GetString("value", "");
			if (s.GetBool("switch", false)) {
				BCheckBox* box = new BCheckBox(key.String(),
					s.GetString("what", ""), NULL);
				box->SetValue(BString(value) == "true"
					? B_CONTROL_ON : B_CONTROL_OFF);
				grid->AddView(box, 1, i);
			} else {
				BTextControl* field = new BTextControl(key.String(),
					s.GetString("what", ""), value, NULL);
				grid->AddItem(field->CreateLabelLayoutItem(), 0, i);
				grid->AddItem(field->CreateTextViewLayoutItem(), 1, i);
				BString unit = s.GetString("unit", "");
				if (!unit.IsEmpty()) {
					BStringView* label = new BStringView(NULL, unit.String());
					label->SetHighUIColor(B_PANEL_TEXT_COLOR, B_LIGHTEN_1_TINT);
					grid->AddView(label, 2, i);
				}
			}
			fSettingKeys.Add(key);
		}
		layout.Add(new BStringView("hint",
			"Filled with the defaults this device gets until you change "
			"them. Saving restarts retifs."))
			.AddGroup(B_HORIZONTAL)
				.AddGlue()
				.Add(new BButton("Save", new BMessage(kSave)))
			.End()
			.AddGlue();
		fSettingsShown = true;
	}

	BString _SettingsJson()
	{
		BString json("{");
		for (int32 i = 0; i < fSettingKeys.CountStrings(); i++) {
			BString key = fSettingKeys.StringAt(i);
			BView* view = FindView(key.String());
			BString value;
			if (BCheckBox* box = dynamic_cast<BCheckBox*>(view))
				value = box->Value() == B_CONTROL_ON ? "true" : "";
			else if (BTextControl* field = dynamic_cast<BTextControl*>(view))
				value = field->Text();
			if (i > 0)
				json << ",";
			json << JsonString(key.String()) << ":" << JsonString(value.String());
		}
		json << "}";
		return json;
	}

	void _Act(const char* op, const BString& a)
	{
		if (a.IsEmpty() && BString(op) != "save")
			return;
		BMessage act(kAct);
		act.AddString("op", op);
		act.AddString("a", a);
		fWorker->PostMessage(&act);
	}

	BString _Selected(BListView* list, const BStringList& values)
	{
		int32 i = list->CurrentSelection();
		return i >= 0 ? values.StringAt(i) : BString();
	}

	void _Show(BMessage* state)
	{
		BMessage status, reticulum, heard, services, settings;
		state->FindMessage("status", &status);
		state->FindMessage("reticulum", &reticulum);
		state->FindMessage("heard", &heard);
		state->FindMessage("services", &services);
		state->FindMessage("settings", &settings);

		// This device.
		if (status.GetBool("up", false)) {
			fState->SetText("retifs is running");
			BString node("Node ");
			node << status.GetString("node", "");
			fNode->SetText(node);
			BMessage usage;
			if (status.FindMessage("usage", &usage) == B_OK) {
				BString line;
				line << "Own data: " << Human(usage.GetDouble("pinned", 0))
					<< " of " << Human(usage.GetDouble("store_limit", 0))
					<< "    Held for others: "
					<< Human(usage.GetDouble("leased", 0));
				fUsage->SetText(line);
				BString line2;
				line2 << "Cache: " << Human(usage.GetDouble("cache", 0))
					<< " of " << Human(usage.GetDouble("cache_max", 0))
					<< "    Free on the disk: "
					<< Human(usage.GetDouble("disk_available", 0));
				fUsage2->SetText(line2);
			}
		} else {
			fState->SetText("retifs is not running");
			fNode->SetText(status.GetString("error", ""));
			fUsage->SetText("");
			fUsage2->SetText("");
		}

		BMessage libraries;
		status.FindMessage("libraries", &libraries);
		_Fill(fLibraries, fLibraryNames, libraries, "name", [](BMessage& l) {
			BString s;
			s << l.GetString("name", "?") << "  —  "
				<< (int32)l.GetDouble("records", 0) << " record(s), "
				<< (int32)l.GetDouble("peers", 0) << " peer(s)"
				<< (l.GetBool("lazy", false) ? ", fetched as needed" : "");
			return s;
		});
		_Fill(fHeard, fHeardAddresses, heard, "address", [](BMessage& l) {
			BString s;
			s << l.GetString("name", "?") << "  —  " << l.GetString("about", "")
				<< "  (" << (int32)l.GetDouble("hops", 0) << " hop(s) away)";
			return s;
		});

		// Network.
		if (reticulum.GetBool("running", false)) {
			BString r("Reticulum is running, transport ");
			r << reticulum.GetString("transport", "?");
			fReticulum->SetText(r);
		} else {
			BString r("Reticulum: ");
			r << reticulum.GetString("error", "not running");
			fReticulum->SetText(r);
		}
		BMessage connections;
		reticulum.FindMessage("connections", &connections);
		_Fill(fConnections, fConnectionNames, connections, "name",
			[](BMessage& c) {
				BString s;
				s << c.GetString("name", "?") << "  —  "
					<< c.GetString("kind", "") << ", "
					<< (!c.GetBool("enabled", false) ? "disabled"
						: c.GetBool("up", false) ? "up" : "down")
					<< "  ↓ " << Human(c.GetDouble("rx", 0))
					<< "  ↑ " << Human(c.GetDouble("tx", 0));
				return s;
			});

		if (!fSettingsShown && Count(settings) > 0)
			_FillSettings(settings);
	}

	template<typename F>
	void _Fill(BListView* list, BStringList& values, const BMessage& array,
		const char* key, F text)
	{
		BString selected;
		int32 current = list->CurrentSelection();
		if (current >= 0)
			selected = values.StringAt(current);
		list->MakeEmpty();
		values.MakeEmpty();
		// An error ({"error": ...}) instead of a list: nothing to show.
		int32 count = array.HasString("error") ? 0 : Count(array);
		for (int32 i = 0; i < count; i++) {
			BMessage item = Item(array, i);
			list->AddItem(new BStringItem(text(item)));
			values.Add(item.GetString(key, ""));
			if (selected == item.GetString(key, ""))
				list->Select(i);
		}
	}

	BTabView* fTabs;
	BStringView* fState;
	BStringView* fNode;
	BStringView* fUsage;
	BStringView* fUsage2;
	BListView* fLibraries;
	BStringList fLibraryNames;
	BListView* fHeard;
	BStringList fHeardAddresses;
	BStringView* fReticulum;
	BListView* fConnections;
	BStringList fConnectionNames;
	BTextControl* fLink;
	BTextControl* fName;
	BTextControl* fHost;
	BTextControl* fPort;
	BView* fSettingsView;
	BStringList fSettingKeys;
	bool fSettingsShown;
	Worker* fWorker;
	BMessageRunner* fRunner;
};


// #pragma mark - Deskbar replicant


/*!	retifs in the Deskbar's tray: the shared status icon (the same as on
	Linux and Windows), and a menu with how things are and what to do.
	Deskbar loads the app's image and calls instantiate_deskbar_item(); the
	view lives in the Deskbar's process, so the core's blocking calls run in
	a thread of its own, which sends the view what it found.
*/
class _EXPORT DeskbarView : public BView {
public:
	DeskbarView(BRect frame)
		:
		BView(frame, kReplicantName, B_FOLLOW_NONE, B_WILL_DRAW),
		fIcon(NULL),
		fHealth("down"),
		fPoller(-1),
		fQuit(false)
	{
	}

	/*! Deskbar keeps its replicants archived and makes them again from the
		archive (finding this app by the "add_on" signature in it). */
	DeskbarView(BMessage* archive)
		:
		BView(archive),
		fIcon(NULL),
		fHealth("down"),
		fPoller(-1),
		fQuit(false)
	{
	}

	~DeskbarView()
	{
		delete fIcon;
	}

	// Defined outside the class: Deskbar looks Instantiate() up by name, and
	// a function defined inside it that nothing here calls isn't emitted.
	static BArchivable* Instantiate(BMessage* archive);
	status_t Archive(BMessage* data, bool deep = true) const;

	void AttachedToWindow()
	{
		BView::AttachedToWindow();
		AdoptParentColors();
		// In the Deskbar's process the core can't tell the app's folder (and
		// retifsd in it) from the program running: say it.
		entry_ref ref;
		BPath app, folder;
		if (be_roster->FindApp(kSignature, &ref) == B_OK
			&& app.SetTo(&ref) == B_OK && app.GetParent(&folder) == B_OK)
			rd_set_app_dir(folder.Path());
		_Draw();
		fPoller = spawn_thread(_Poll, "retifs tray", B_LOW_PRIORITY, this);
		resume_thread(fPoller);
	}

	void DetachedFromWindow()
	{
		fQuit = true;
		status_t ignored;
		wait_for_thread(fPoller, &ignored);
		BView::DetachedFromWindow();
	}

	void Draw(BRect)
	{
		if (fIcon == NULL)
			return;
		SetDrawingMode(B_OP_ALPHA);
		SetBlendingMode(B_PIXEL_ALPHA, B_ALPHA_OVERLAY);
		DrawBitmap(fIcon, Bounds().LeftTop()
			+ BPoint((Bounds().Width() - fIcon->Bounds().Width()) / 2,
				(Bounds().Height() - fIcon->Bounds().Height()) / 2));
	}

	void MessageReceived(BMessage* message)
	{
		switch (message->what) {
			case kTray:
			{
				BString health = message->GetString("health", "down");
				fLines.MakeEmpty();
				BMessage lines;
				if (message->FindMessage("lines", &lines) == B_OK) {
					for (int32 i = 0; i < Count(lines); i++) {
						BString name;
						name << i;
						fLines.Add(lines.GetString(name.String(), ""));
					}
				}
				if (health != fHealth) {
					fHealth = health;
					_Draw();
					Invalidate();
				}
				break;
			}
			case kOpen:
				be_roster->Launch(kSignature);
				break;
			case kRestart:
				_Detached(_Restart);
				break;
			case kRemove:
				_Detached(_Remove);
				break;
			default:
				BView::MessageReceived(message);
		}
	}

	void MouseDown(BPoint where)
	{
		BPopUpMenu* menu = new BPopUpMenu("retifs", false, false);
		for (int32 i = 0; i < fLines.CountStrings(); i++) {
			BMenuItem* item = new BMenuItem(fLines.StringAt(i).String(), NULL);
			item->SetEnabled(false);
			menu->AddItem(item);
		}
		if (fLines.CountStrings() > 0)
			menu->AddSeparatorItem();
		menu->AddItem(new BMenuItem("Open retifs" B_UTF8_ELLIPSIS,
			new BMessage(kOpen)));
		menu->AddItem(new BMenuItem("Restart retifs", new BMessage(kRestart)));
		menu->AddSeparatorItem();
		menu->AddItem(new BMenuItem("Remove from Deskbar",
			new BMessage(kRemove)));
		menu->SetTargetForItems(this);
		menu->SetAsyncAutoDestruct(true);
		menu->Go(ConvertToScreen(where), true, true, true);
	}

private:
	enum {
		kTray = 'tray',
		kOpen = 'open',
		kRestart = 'rsta',
		kRemove = 'remo',
	};

	void _Draw()
	{
		int32 size = (int32)Bounds().Height() + 1;
		if (size < 8)
			size = 16;
		if (fIcon == NULL || fIcon->Bounds().IntegerHeight() + 1 != size) {
			delete fIcon;
			fIcon = new BBitmap(BRect(0, 0, size - 1, size - 1), B_RGBA32);
		}
		// ARGB in a u32 per pixel is B_RGBA32's layout on little-endian x86.
		rd_icon(size, fHealth.String(), (unsigned int*)fIcon->Bits());
	}

	static status_t _Poll(void* data)
	{
		DeskbarView* view = (DeskbarView*)data;
		BMessenger target(view);
		while (!view->fQuit) {
			BMessage state = Take(rd_tray());
			state.what = kTray;
			target.SendMessage(&state);
			for (int i = 0; i < 50 && !view->fQuit; i++)
				snooze(100000);
		}
		return B_OK;
	}

	static void _Detached(thread_func f)
	{
		thread_id t = spawn_thread(f, "retifs tray action", B_NORMAL_PRIORITY,
			NULL);
		resume_thread(t);
	}

	static status_t _Restart(void*)
	{
		rd_free(rd_service(kDaemonJob, "restart"));
		return B_OK;
	}

	static status_t _Remove(void*)
	{
		BDeskbar().RemoveItem(kReplicantName);
		return B_OK;
	}

	BBitmap* fIcon;
	BString fHealth;
	BStringList fLines;
	thread_id fPoller;
	volatile bool fQuit;
};


BArchivable*
DeskbarView::Instantiate(BMessage* archive)
{
	if (!validate_instantiation(archive, "DeskbarView"))
		return NULL;
	return new DeskbarView(archive);
}


status_t
DeskbarView::Archive(BMessage* data, bool deep) const
{
	status_t status = BView::Archive(data, deep);
	if (status == B_OK)
		status = data->AddString("add_on", kSignature);
	if (status == B_OK)
		status = data->AddString("class", "DeskbarView");
	return status;
}


extern "C" _EXPORT BView*
instantiate_deskbar_item(float maxWidth, float maxHeight)
{
	return new DeskbarView(BRect(0, 0, maxHeight - 1, maxHeight - 1));
}


/*!	Puts the replicant in the Deskbar, unless it is there already: Deskbar
	loads it from this app's file, and keeps it there across restarts.
*/
static void
InstallInDeskbar()
{
	BDeskbar deskbar;
	// At login (the package's launch job) the Deskbar may not be up yet.
	for (int i = 0; i < 30 && !deskbar.IsRunning(); i++)
		snooze(1000000);
	if (!deskbar.IsRunning() || deskbar.HasItem(kReplicantName))
		return;
	app_info info;
	if (be_app->GetAppInfo(&info) == B_OK)
		deskbar.AddItem(&info.ref);
}


// #pragma mark - App


class RetifsApp : public BApplication {
public:
	RetifsApp()
		:
		BApplication(kSignature),
		fDeskbarOnly(false)
	{
	}

	void ArgvReceived(int32 argc, char** argv)
	{
		for (int32 i = 1; i < argc; i++) {
			if (strcmp(argv[i], "--deskbar") == 0)
				fDeskbarOnly = true;
		}
	}

	void ReadyToRun()
	{
		InstallInDeskbar();
		if (fDeskbarOnly) {
			Quit();
			return;
		}
		(new RetifsWindow())->Show();
	}

private:
	bool fDeskbarOnly;
};


int
main()
{
	RetifsApp app;
	app.Run();
	return 0;
}

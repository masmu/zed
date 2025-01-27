use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use axum::{extract, routing::post, Extension, Json, Router};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use stripe::{
    BillingPortalSession, CheckoutSession, CreateBillingPortalSession,
    CreateBillingPortalSessionFlowData, CreateBillingPortalSessionFlowDataAfterCompletion,
    CreateBillingPortalSessionFlowDataAfterCompletionRedirect,
    CreateBillingPortalSessionFlowDataType, CreateCheckoutSession, CreateCheckoutSessionLineItems,
    CreateCustomer, Customer, CustomerId, EventObject, EventType, Expandable, ListEvents,
    SubscriptionStatus,
};
use util::ResultExt;

use crate::db::billing_subscription::StripeSubscriptionStatus;
use crate::db::{
    billing_customer, BillingSubscriptionId, CreateBillingCustomerParams,
    CreateBillingSubscriptionParams,
};
use crate::{AppState, Error, Result};

pub fn router() -> Router {
    Router::new()
        .route("/billing/subscriptions", post(create_billing_subscription))
        .route(
            "/billing/subscriptions/manage",
            post(manage_billing_subscription),
        )
}

#[derive(Debug, Deserialize)]
struct CreateBillingSubscriptionBody {
    github_user_id: i32,
}

#[derive(Debug, Serialize)]
struct CreateBillingSubscriptionResponse {
    checkout_session_url: String,
}

/// Initiates a Stripe Checkout session for creating a billing subscription.
async fn create_billing_subscription(
    Extension(app): Extension<Arc<AppState>>,
    extract::Json(body): extract::Json<CreateBillingSubscriptionBody>,
) -> Result<Json<CreateBillingSubscriptionResponse>> {
    let user = app
        .db
        .get_user_by_github_user_id(body.github_user_id)
        .await?
        .ok_or_else(|| anyhow!("user not found"))?;

    let Some((stripe_client, stripe_price_id)) = app
        .stripe_client
        .clone()
        .zip(app.config.stripe_price_id.clone())
    else {
        log::error!("failed to retrieve Stripe client or price ID");
        Err(Error::Http(
            StatusCode::NOT_IMPLEMENTED,
            "not supported".into(),
        ))?
    };

    let customer_id =
        if let Some(existing_customer) = app.db.get_billing_customer_by_user_id(user.id).await? {
            CustomerId::from_str(&existing_customer.stripe_customer_id)
                .context("failed to parse customer ID")?
        } else {
            let customer = Customer::create(
                &stripe_client,
                CreateCustomer {
                    email: user.email_address.as_deref(),
                    ..Default::default()
                },
            )
            .await?;

            customer.id
        };

    let checkout_session = {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(stripe::CheckoutSessionMode::Subscription);
        params.customer = Some(customer_id);
        params.client_reference_id = Some(user.github_login.as_str());
        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            price: Some(stripe_price_id.to_string()),
            quantity: Some(1),
            ..Default::default()
        }]);
        params.success_url = Some("https://zed.dev/billing/success");

        CheckoutSession::create(&stripe_client, params).await?
    };

    Ok(Json(CreateBillingSubscriptionResponse {
        checkout_session_url: checkout_session
            .url
            .ok_or_else(|| anyhow!("no checkout session URL"))?,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ManageSubscriptionIntent {
    /// The user intends to cancel their subscription.
    Cancel,
}

#[derive(Debug, Deserialize)]
struct ManageBillingSubscriptionBody {
    github_user_id: i32,
    intent: ManageSubscriptionIntent,
    /// The ID of the subscription to manage.
    ///
    /// If not provided, we will try to use the active subscription (if there is only one).
    subscription_id: Option<BillingSubscriptionId>,
}

#[derive(Debug, Serialize)]
struct ManageBillingSubscriptionResponse {
    billing_portal_session_url: String,
}

/// Initiates a Stripe customer portal session for managing a billing subscription.
async fn manage_billing_subscription(
    Extension(app): Extension<Arc<AppState>>,
    extract::Json(body): extract::Json<ManageBillingSubscriptionBody>,
) -> Result<Json<ManageBillingSubscriptionResponse>> {
    let user = app
        .db
        .get_user_by_github_user_id(body.github_user_id)
        .await?
        .ok_or_else(|| anyhow!("user not found"))?;

    let Some(stripe_client) = app.stripe_client.clone() else {
        log::error!("failed to retrieve Stripe client");
        Err(Error::Http(
            StatusCode::NOT_IMPLEMENTED,
            "not supported".into(),
        ))?
    };

    let customer = app
        .db
        .get_billing_customer_by_user_id(user.id)
        .await?
        .ok_or_else(|| anyhow!("billing customer not found"))?;
    let customer_id = CustomerId::from_str(&customer.stripe_customer_id)
        .context("failed to parse customer ID")?;

    let subscription = if let Some(subscription_id) = body.subscription_id {
        app.db
            .get_billing_subscription_by_id(subscription_id)
            .await?
            .ok_or_else(|| anyhow!("subscription not found"))?
    } else {
        // If no subscription ID was provided, try to find the only active subscription ID.
        let subscriptions = app.db.get_active_billing_subscriptions(user.id).await?;
        if subscriptions.len() > 1 {
            Err(anyhow!("user has multiple active subscriptions"))?;
        }

        subscriptions
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("user has no active subscriptions"))?
    };

    let flow = match body.intent {
        ManageSubscriptionIntent::Cancel => CreateBillingPortalSessionFlowData {
            type_: CreateBillingPortalSessionFlowDataType::SubscriptionCancel,
            after_completion: Some(CreateBillingPortalSessionFlowDataAfterCompletion {
                type_: stripe::CreateBillingPortalSessionFlowDataAfterCompletionType::Redirect,
                redirect: Some(CreateBillingPortalSessionFlowDataAfterCompletionRedirect {
                    return_url: "https://zed.dev/billing".into(),
                }),
                ..Default::default()
            }),
            subscription_cancel: Some(
                stripe::CreateBillingPortalSessionFlowDataSubscriptionCancel {
                    subscription: subscription.stripe_subscription_id,
                    retention: None,
                },
            ),
            ..Default::default()
        },
    };

    let mut params = CreateBillingPortalSession::new(customer_id);
    params.flow_data = Some(flow);
    params.return_url = Some("https://zed.dev/billing");

    let session = BillingPortalSession::create(&stripe_client, params).await?;

    Ok(Json(ManageBillingSubscriptionResponse {
        billing_portal_session_url: session.url,
    }))
}

const POLL_EVENTS_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Polls the Stripe events API periodically to reconcile the records in our
/// database with the data in Stripe.
pub fn poll_stripe_events_periodically(app: Arc<AppState>) {
    let Some(stripe_client) = app.stripe_client.clone() else {
        log::warn!("failed to retrieve Stripe client");
        return;
    };

    let executor = app.executor.clone();
    executor.spawn_detached({
        let executor = executor.clone();
        async move {
            loop {
                poll_stripe_events(&app, &stripe_client).await.log_err();

                executor.sleep(POLL_EVENTS_INTERVAL).await;
            }
        }
    });
}

async fn poll_stripe_events(
    app: &Arc<AppState>,
    stripe_client: &stripe::Client,
) -> anyhow::Result<()> {
    let event_types = [
        EventType::CustomerCreated.to_string(),
        EventType::CustomerSubscriptionCreated.to_string(),
        EventType::CustomerSubscriptionUpdated.to_string(),
        EventType::CustomerSubscriptionPaused.to_string(),
        EventType::CustomerSubscriptionResumed.to_string(),
        EventType::CustomerSubscriptionDeleted.to_string(),
    ]
    .into_iter()
    .map(|event_type| {
        // Calling `to_string` on `stripe::EventType` members gives us a quoted string,
        // so we need to unquote it.
        event_type.trim_matches('"').to_string()
    })
    .collect::<Vec<_>>();

    loop {
        log::info!("retrieving events from Stripe: {}", event_types.join(", "));

        let mut params = ListEvents::new();
        params.types = Some(event_types.clone());
        params.limit = Some(100);

        let events = stripe::Event::list(stripe_client, &params).await?;
        for event in events.data {
            match event.type_ {
                EventType::CustomerCreated => {
                    handle_customer_event(app, stripe_client, event)
                        .await
                        .log_err();
                }
                EventType::CustomerSubscriptionCreated
                | EventType::CustomerSubscriptionUpdated
                | EventType::CustomerSubscriptionPaused
                | EventType::CustomerSubscriptionResumed
                | EventType::CustomerSubscriptionDeleted => {
                    handle_customer_subscription_event(app, stripe_client, event)
                        .await
                        .log_err();
                }
                _ => {}
            }
        }

        if !events.has_more {
            break;
        }
    }

    Ok(())
}

async fn handle_customer_event(
    app: &Arc<AppState>,
    stripe_client: &stripe::Client,
    event: stripe::Event,
) -> anyhow::Result<()> {
    let EventObject::Customer(customer) = event.data.object else {
        bail!("unexpected event payload for {}", event.id);
    };

    find_or_create_billing_customer(app, stripe_client, Expandable::Object(Box::new(customer)))
        .await?;

    Ok(())
}

async fn handle_customer_subscription_event(
    app: &Arc<AppState>,
    stripe_client: &stripe::Client,
    event: stripe::Event,
) -> anyhow::Result<()> {
    let EventObject::Subscription(subscription) = event.data.object else {
        bail!("unexpected event payload for {}", event.id);
    };

    let billing_customer =
        find_or_create_billing_customer(app, stripe_client, subscription.customer)
            .await?
            .ok_or_else(|| anyhow!("billing customer not found"))?;

    app.db
        .upsert_billing_subscription_by_stripe_subscription_id(&CreateBillingSubscriptionParams {
            billing_customer_id: billing_customer.id,
            stripe_subscription_id: subscription.id.to_string(),
            stripe_subscription_status: subscription.status.into(),
        })
        .await?;

    Ok(())
}

impl From<SubscriptionStatus> for StripeSubscriptionStatus {
    fn from(value: SubscriptionStatus) -> Self {
        match value {
            SubscriptionStatus::Incomplete => Self::Incomplete,
            SubscriptionStatus::IncompleteExpired => Self::IncompleteExpired,
            SubscriptionStatus::Trialing => Self::Trialing,
            SubscriptionStatus::Active => Self::Active,
            SubscriptionStatus::PastDue => Self::PastDue,
            SubscriptionStatus::Canceled => Self::Canceled,
            SubscriptionStatus::Unpaid => Self::Unpaid,
            SubscriptionStatus::Paused => Self::Paused,
        }
    }
}

/// Finds or creates a billing customer using the provided customer.
async fn find_or_create_billing_customer(
    app: &Arc<AppState>,
    stripe_client: &stripe::Client,
    customer_or_id: Expandable<Customer>,
) -> anyhow::Result<Option<billing_customer::Model>> {
    let customer_id = match &customer_or_id {
        Expandable::Id(id) => id,
        Expandable::Object(customer) => customer.id.as_ref(),
    };

    // If we already have a billing customer record associated with the Stripe customer,
    // there's nothing more we need to do.
    if let Some(billing_customer) = app
        .db
        .get_billing_customer_by_stripe_customer_id(&customer_id)
        .await?
    {
        return Ok(Some(billing_customer));
    }

    // If all we have is a customer ID, resolve it to a full customer record by
    // hitting the Stripe API.
    let customer = match customer_or_id {
        Expandable::Id(id) => Customer::retrieve(&stripe_client, &id, &[]).await?,
        Expandable::Object(customer) => *customer,
    };

    let Some(email) = customer.email else {
        return Ok(None);
    };

    let Some(user) = app.db.get_user_by_email(&email).await? else {
        return Ok(None);
    };

    let billing_customer = app
        .db
        .create_billing_customer(&CreateBillingCustomerParams {
            user_id: user.id,
            stripe_customer_id: customer.id.to_string(),
        })
        .await?;

    Ok(Some(billing_customer))
}
